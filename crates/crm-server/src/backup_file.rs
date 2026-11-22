//! Файл резервной копии: потоковое сжатие zstd и, по желанию владельца, шифрование
//! паролем. Память не зависит от размера базы — данные идут кусками.
//!
//! Формат зашифрованной копии: `CRMENC01`, соль (16 байт), начало nonce (19 байт),
//! дальше куски `[длина u32 LE][шифротекст]` (XChaCha20-Poly1305, поток STREAM;
//! последний кусок помечен, поэтому обрезанный файл не пройдёт проверку).
//! Незашифрованная копия — просто поток zstd, как в прежних версиях.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};

const MAGIC: &[u8; 8] = b"CRMENC01";
const SALT: usize = 16;
const NONCE: usize = 19;
const CHUNK: usize = 1 << 20;
/// Ключ шифрования копий. Лежит рядом с базой, но не внутри неё — в копию не попадает.
pub const KEY_FILE: &str = "backup.key";
const LEVEL: i32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum UnpackError {
    #[error("копия зашифрована — введите пароль копий")]
    NeedPassword,
    #[error("неверный пароль копий или файл повреждён")]
    WrongPassword,
    #[error("это не копия CRM или файл повреждён")]
    NotBackup,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Ключ из пароля. Соль общая для всех копий одного пароля.
#[derive(Clone)]
pub struct BackupKey {
    salt: [u8; SALT],
    key: [u8; 32],
}

impl std::fmt::Debug for BackupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackupKey(..)")
    }
}

impl BackupKey {
    pub fn from_password(password: &str) -> io::Result<Self> {
        let mut salt = [0u8; SALT];
        getrandom::fill(&mut salt).map_err(io::Error::other)?;
        Self::derive(password, salt)
    }

    fn derive(password: &str, salt: [u8; SALT]) -> io::Result<Self> {
        let mut key = [0u8; 32];
        argon2::Argon2::default()
            .hash_password_into(password.as_bytes(), &salt, &mut key)
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self { salt, key })
    }

    /// Ключ этого сервера, если владелец включил шифрование.
    pub fn load(db_dir: &Path) -> io::Result<Option<Self>> {
        match std::fs::read(db_dir.join(KEY_FILE)) {
            Ok(b) if b.len() == SALT + 32 => {
                let mut k = Self {
                    salt: [0; SALT],
                    key: [0; 32],
                };
                k.salt.copy_from_slice(&b[..SALT]);
                k.key.copy_from_slice(&b[SALT..]);
                Ok(Some(k))
            }
            Ok(_) => Err(io::Error::other("файл ключа копий повреждён")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, db_dir: &Path) -> io::Result<()> {
        let tmp = db_dir.join(format!("{KEY_FILE}.tmp"));
        let mut b = self.salt.to_vec();
        b.extend_from_slice(&self.key);
        std::fs::write(&tmp, b)?;
        std::fs::rename(tmp, db_dir.join(KEY_FILE))
    }

    pub fn remove(db_dir: &Path) -> io::Result<()> {
        match std::fs::remove_file(db_dir.join(KEY_FILE)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.key).into())
    }
}

/// Сжать (и зашифровать) файл базы в копию. Возвращает размер копии.
pub fn pack(src: &Path, dst: &Path, key: Option<&BackupKey>) -> io::Result<u64> {
    let input = BufReader::new(File::open(src)?);
    let out = BufWriter::new(File::create(dst)?);
    match key {
        None => {
            let mut out = out;
            zstd::stream::copy_encode(input, &mut out, LEVEL)?;
            out.flush()?;
        }
        Some(k) => {
            let mut out = out;
            let mut nonce = [0u8; NONCE];
            getrandom::fill(&mut nonce).map_err(io::Error::other)?;
            out.write_all(MAGIC)?;
            out.write_all(&k.salt)?;
            out.write_all(&nonce)?;
            let mut enc = EncWriter {
                enc: Some(EncryptorBE32::from_aead(k.cipher(), (&nonce).into())),
                buf: Vec::with_capacity(CHUNK + 1),
                out,
            };
            zstd::stream::copy_encode(input, &mut enc, LEVEL)?;
            enc.finish()?;
        }
    }
    Ok(std::fs::metadata(dst)?.len())
}

/// Зашифрована ли копия (по заголовку).
pub fn is_encrypted(src: &Path) -> io::Result<bool> {
    let mut head = [0u8; 8];
    let mut f = File::open(src)?;
    Ok(f.read_exact(&mut head).is_ok() && &head == MAGIC)
}

/// Распаковать копию в файл базы. Зашифрованную открывает ключ этого сервера
/// (если пароль тот же) или введённый пароль.
pub fn unpack(
    src: &Path,
    dst: &Path,
    local: Option<&BackupKey>,
    password: Option<&str>,
) -> Result<(), UnpackError> {
    let mut input = BufReader::new(File::open(src)?);
    let mut out = BufWriter::new(File::create(dst)?);
    if !is_encrypted(src)? {
        zstd::stream::copy_decode(input, &mut out).map_err(|_| UnpackError::NotBackup)?;
        out.flush()?;
        return Ok(());
    }
    let (mut magic, mut salt, mut nonce) = ([0u8; 8], [0u8; SALT], [0u8; NONCE]);
    for part in [&mut magic[..], &mut salt[..], &mut nonce[..]] {
        input.read_exact(part).map_err(|_| UnpackError::NotBackup)?;
    }
    let key = match (local.filter(|k| k.salt == salt), password) {
        (Some(k), _) => k.clone(),
        (None, Some(p)) => BackupKey::derive(p, salt)?,
        (None, None) => return Err(UnpackError::NeedPassword),
    };
    let dec = DecReader {
        dec: Some(DecryptorBE32::from_aead(key.cipher(), (&nonce).into())),
        input,
        plain: Vec::new(),
        pos: 0,
        pending: None,
    };
    match zstd::stream::copy_decode(dec, &mut out) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::InvalidData && e.to_string() == BAD_TAG => {
            return Err(UnpackError::WrongPassword)
        }
        Err(_) => return Err(UnpackError::NotBackup),
    }
    out.flush()?;
    Ok(())
}

const BAD_TAG: &str = "шифр не сошёлся";

struct EncWriter<W: Write> {
    enc: Option<EncryptorBE32<XChaCha20Poly1305>>,
    buf: Vec<u8>,
    out: W,
}

impl<W: Write> EncWriter<W> {
    fn emit(&mut self, ct: &[u8]) -> io::Result<()> {
        let len = u32::try_from(ct.len()).map_err(io::Error::other)?;
        self.out.write_all(&len.to_le_bytes())?;
        self.out.write_all(ct)
    }

    fn finish(mut self) -> io::Result<()> {
        let enc = self
            .enc
            .take()
            .ok_or_else(|| io::Error::other("уже закрыт"))?;
        let ct = enc
            .encrypt_last(self.buf.as_slice())
            .map_err(|_| io::Error::other("шифрование"))?;
        self.emit(&ct)?;
        self.out.flush()
    }
}

impl<W: Write> Write for EncWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        // Кусок уходит, только когда за ним точно есть ещё данные: последний — в finish.
        while self.buf.len() > CHUNK {
            let rest = self.buf.split_off(CHUNK);
            let chunk = std::mem::replace(&mut self.buf, rest);
            let ct = self
                .enc
                .as_mut()
                .ok_or_else(|| io::Error::other("уже закрыт"))?
                .encrypt_next(chunk.as_slice())
                .map_err(|_| io::Error::other("шифрование"))?;
            self.emit(&ct)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct DecReader<R: Read> {
    dec: Option<DecryptorBE32<XChaCha20Poly1305>>,
    input: R,
    plain: Vec<u8>,
    pos: usize,
    /// Прочитанный наперёд кусок: по нему видно, что текущий — не последний.
    pending: Option<Vec<u8>>,
}

impl<R: Read> DecReader<R> {
    fn read_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut len = [0u8; 4];
        match self.input.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let len = u32::from_le_bytes(len) as usize;
        if len > CHUNK + 16 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, BAD_TAG));
        }
        let mut ct = vec![0u8; len];
        self.input.read_exact(&mut ct)?;
        Ok(Some(ct))
    }

    fn refill(&mut self) -> io::Result<()> {
        let Some(mut dec) = self.dec.take() else {
            return Ok(()); // последний кусок уже расшифрован
        };
        let bad = || io::Error::new(io::ErrorKind::InvalidData, BAD_TAG);
        let current = match self.pending.take() {
            Some(c) => c,
            None => self.read_chunk()?.ok_or_else(bad)?,
        };
        let next = self.read_chunk()?;
        self.plain = if next.is_some() {
            let p = dec.decrypt_next(current.as_slice()).map_err(|_| bad())?;
            self.pending = next;
            self.dec = Some(dec);
            p
        } else {
            dec.decrypt_last(current.as_slice()).map_err(|_| bad())?
        };
        self.pos = 0;
        Ok(())
    }
}

impl<R: Read> Read for DecReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.plain.len() {
            if self.dec.is_none() {
                return Ok(0);
            }
            self.refill()?;
        }
        let n = out.len().min(self.plain.len() - self.pos);
        out[..n].copy_from_slice(&self.plain[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path, len: usize) -> std::path::PathBuf {
        // Плохо сжимаемые данные: копия займёт несколько кусков шифра.
        let mut x: u64 = 7;
        let data: Vec<u8> = (0..len)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                (x >> 33) as u8
            })
            .collect();
        let p = dir.join("db");
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn plain_copy_round_trips_and_old_copies_still_open() {
        let d = tempfile::tempdir().unwrap();
        let db = sample(d.path(), 3 * CHUNK + 5);
        pack(&db, &d.path().join("c"), None).unwrap();
        assert!(!is_encrypted(&d.path().join("c")).unwrap());
        unpack(&d.path().join("c"), &d.path().join("out"), None, None).unwrap();
        assert_eq!(
            std::fs::read(&db).unwrap(),
            std::fs::read(d.path().join("out")).unwrap()
        );

        // Копия прежних версий: zstd::encode_all целиком.
        let old = zstd::encode_all(std::fs::read(&db).unwrap().as_slice(), 3).unwrap();
        std::fs::write(d.path().join("old"), old).unwrap();
        unpack(&d.path().join("old"), &d.path().join("out2"), None, None).unwrap();
        assert_eq!(
            std::fs::read(&db).unwrap(),
            std::fs::read(d.path().join("out2")).unwrap()
        );
    }

    #[test]
    fn encrypted_copy_needs_the_password() {
        let d = tempfile::tempdir().unwrap();
        let db = sample(d.path(), 3 * CHUNK + 5);
        let key = BackupKey::from_password("секретный пароль").unwrap();
        let c = d.path().join("c");
        pack(&db, &c, Some(&key)).unwrap();
        assert!(is_encrypted(&c).unwrap());
        let raw = std::fs::read(&c).unwrap();
        let probe = &std::fs::read(&db).unwrap()[..64];
        assert!(
            !raw.windows(64).any(|w| w == probe),
            "данные не лежат открыто"
        );

        let out = d.path().join("out");
        assert!(matches!(
            unpack(&c, &out, None, None),
            Err(UnpackError::NeedPassword)
        ));
        assert!(matches!(
            unpack(&c, &out, None, Some("не тот")),
            Err(UnpackError::WrongPassword)
        ));
        unpack(&c, &out, None, Some("секретный пароль")).unwrap();
        assert_eq!(std::fs::read(&db).unwrap(), std::fs::read(&out).unwrap());
        // На этом сервере пароль не нужен — есть ключ.
        unpack(&c, &out, Some(&key), None).unwrap();
        // Ключ другого пароля не подходит — просим пароль.
        let other = BackupKey::from_password("другой").unwrap();
        assert!(matches!(
            unpack(&c, &out, Some(&other), None),
            Err(UnpackError::NeedPassword)
        ));
    }

    #[test]
    fn truncated_or_tampered_copy_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let db = sample(d.path(), 2 * CHUNK + 5);
        let key = BackupKey::from_password("пароль").unwrap();
        let c = d.path().join("c");
        pack(&db, &c, Some(&key)).unwrap();
        let raw = std::fs::read(&c).unwrap();
        let out = d.path().join("out");
        // Отрезан последний кусок: целые куски сходятся, но конца нет.
        let first = 8 + SALT + NONCE;
        let len = u32::from_le_bytes(raw[first..first + 4].try_into().unwrap()) as usize;
        std::fs::write(&c, &raw[..first + 4 + len]).unwrap();
        assert!(
            unpack(&c, &out, Some(&key), None).is_err(),
            "обрезанная копия"
        );
        let mut bad = raw.clone();
        bad[first + 10] ^= 1;
        std::fs::write(&c, bad).unwrap();
        assert!(
            unpack(&c, &out, Some(&key), None).is_err(),
            "изменённая копия"
        );
    }

    #[test]
    fn key_file_round_trips() {
        let d = tempfile::tempdir().unwrap();
        assert!(BackupKey::load(d.path()).unwrap().is_none());
        let k = BackupKey::from_password("p").unwrap();
        k.save(d.path()).unwrap();
        let back = BackupKey::load(d.path()).unwrap().unwrap();
        assert_eq!((back.salt, back.key), (k.salt, k.key));
        BackupKey::remove(d.path()).unwrap();
        BackupKey::remove(d.path()).unwrap();
        assert!(BackupKey::load(d.path()).unwrap().is_none());
    }
}

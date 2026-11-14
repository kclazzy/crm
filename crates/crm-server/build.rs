//! Если веб-интерфейс ещё не собран (`web/dist`), кладём заглушку, чтобы сервер
//! собирался и без Node.js. Настоящая сборка: `npm run build` в папке web.
use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    let index = dist.join("index.html");
    println!("cargo:rerun-if-changed={}", index.display());
    if !index.exists() {
        let page = "<!doctype html><html lang=\"ru\"><meta charset=\"utf-8\"><title>CRM</title>\
                    <div id=\"root\">Веб-интерфейс не собран: выполните npm run build в папке web.</div></html>";
        if std::fs::create_dir_all(&dist)
            .and_then(|()| std::fs::write(&index, page))
            .is_err()
        {
            println!(
                "cargo:warning=не удалось создать заглушку {}",
                index.display()
            );
        }
    }
}

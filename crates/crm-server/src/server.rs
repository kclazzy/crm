//! Запуск и остановка сервера: БД, HTTPS, корректное завершение.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::http::{router, AppState};
use crate::{db, tls};

/// Как часто проверять, не сменились ли адреса компьютера (нужен новый сертификат).
const TLS_CHECK_EVERY: Duration = Duration::from_secs(600);
/// Сколько ждать завершения текущих запросов при остановке.
const GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Db(#[from] db::DbError),
    #[error(transparent)]
    Tls(#[from] tls::TlsError),
    #[error("не удалось загрузить сертификат: {0}")]
    TlsLoad(std::io::Error),
    #[error("не удалось открыть порт {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("сервер завершился с ошибкой: {0}")]
    Serve(std::io::Error),
    #[error("сервер аварийно завершился: {0}")]
    Join(#[from] tokio::task::JoinError),
}

pub struct Running {
    pub addr: SocketAddr,
    pub ca_fingerprint: Option<String>,
    handle: Handle,
    serve: JoinHandle<std::io::Result<()>>,
    renew: Option<JoinHandle<()>>,
}

pub async fn start(cfg: &Config) -> Result<Running, RunError> {
    let pool = db::open(&cfg.storage).await?;
    let tls_dir = cfg.storage.db_dir.join("tls");
    let material = if cfg.tls.enabled {
        let dir = tls_dir.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                tls::ensure(&dir, &tls::local_sans(), time::OffsetDateTime::now_utc())
            })
            .await??,
        )
    } else {
        None
    };

    let state = AppState {
        pool,
        ca_pem: material.as_ref().map(|m| Arc::from(m.ca_pem.as_str())),
    };
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    let handle = Handle::new();

    let (serve, renew) = match &material {
        Some(m) => {
            let rustls = RustlsConfig::from_pem(
                m.cert_pem.clone().into_bytes(),
                m.key_pem.clone().into_bytes(),
            )
            .await
            .map_err(RunError::TlsLoad)?;
            let renew = tokio::spawn(keep_certificate_fresh(
                tls_dir,
                rustls.clone(),
                m.cert_pem.clone(),
            ));
            let server = axum_server::bind_rustls(cfg.listen, rustls).handle(handle.clone());
            (tokio::spawn(server.serve(app)), Some(renew))
        }
        None => {
            let server = axum_server::bind(cfg.listen).handle(handle.clone());
            (tokio::spawn(server.serve(app)), None)
        }
    };

    let Some(addr) = handle.listening().await else {
        let source = match serve.await? {
            Err(e) => e,
            Ok(()) => std::io::Error::other("сервер остановился до начала работы"),
        };
        return Err(RunError::Bind {
            addr: cfg.listen,
            source,
        });
    };
    Ok(Running {
        addr,
        ca_fingerprint: material.map(|m| m.ca_fingerprint),
        handle,
        serve,
        renew,
    })
}

/// Если у компьютера появился новый адрес или сертификат скоро истечёт —
/// выпускаем новый и подменяем без перезапуска.
async fn keep_certificate_fresh(
    dir: std::path::PathBuf,
    rustls: RustlsConfig,
    mut current: String,
) {
    loop {
        tokio::time::sleep(TLS_CHECK_EVERY).await;
        let d = dir.clone();
        let fresh = tokio::task::spawn_blocking(move || {
            tls::ensure(&d, &tls::local_sans(), time::OffsetDateTime::now_utc())
        })
        .await;
        match fresh {
            Ok(Ok(m)) if m.cert_pem != current => {
                match rustls
                    .reload_from_pem(m.cert_pem.clone().into_bytes(), m.key_pem.into_bytes())
                    .await
                {
                    Ok(()) => {
                        tracing::info!("сертификат сервера обновлён");
                        current = m.cert_pem;
                    }
                    Err(e) => tracing::error!(error = %e, "не удалось применить новый сертификат"),
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::error!(error = %e, "проверка сертификата"),
            Err(e) => tracing::error!(error = %e, "проверка сертификата прервана"),
        }
    }
}

impl Running {
    /// Корректная остановка: новые соединения не принимаются, текущие запросы дорабатывают.
    pub async fn stop(self) -> Result<(), RunError> {
        self.handle.graceful_shutdown(Some(GRACE));
        if let Some(r) = self.renew {
            r.abort();
        }
        self.serve.await?.map_err(RunError::Serve)
    }

    /// Ждать, пока сервер не остановится сам (ошибка) или не придёт сигнал `shutdown`.
    pub async fn run_until(
        self,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> Result<(), RunError> {
        shutdown.await;
        self.stop().await
    }
}

//! Built-in database for the Personal edition.
//!
//! When no database is configured, CrewConvo downloads PostgreSQL once (into the data directory),
//! initializes a private instance there and runs it as a child process on a local port. Nothing
//! has to be installed or configured; all data stays in `<DATA_DIR>/postgres`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ac_decider::secret::random_bytes;
use anyhow::{Context, Result};
use postgresql_embedded::{PostgreSQL, Settings, Status};

const DATABASE: &str = "crewconvo";

pub struct EmbeddedDb {
    pg: PostgreSQL,
}

fn read_or_create(path: &Path, make: impl FnOnce() -> String) -> Result<String> {
    if let Ok(s) = std::fs::read_to_string(path) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    let value = make();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, &value).with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(value)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The key that encrypts stored secrets, kept next to the data so they survive restarts.
pub fn master_key(data_dir: &Path) -> Result<String> {
    use base64::Engine;
    read_or_create(&data_dir.join("master.key"), || {
        base64::engine::general_purpose::STANDARD.encode(random_bytes::<32>())
    })
}

impl EmbeddedDb {
    /// Installs (first run only) and starts the built-in database. Returns it with its URL.
    pub async fn start(data_dir: &Path) -> Result<(Self, String)> {
        let root: PathBuf = std::path::absolute(data_dir)?.join("postgres");
        std::fs::create_dir_all(&root)?;
        let password = read_or_create(&root.join("password"), || hex(&random_bytes::<16>()))?;

        let mut settings = Settings::new();
        settings.installation_dir = root.join("bin");
        settings.data_dir = root.join("data");
        settings.password_file = root.join("pgpass");
        settings.password = password;
        settings.host = "127.0.0.1".into();
        settings.temporary = false;
        settings.timeout = Some(Duration::from_secs(120));
        if let Some(port) = std::env::var("EMBEDDED_DB_PORT").ok().and_then(|p| p.trim().parse().ok()) {
            settings.port = port;
        }

        let mut pg = PostgreSQL::new(settings);
        if pg.status() == Status::NotInstalled {
            tracing::info!(dir = %root.display(), "first run: downloading the built-in database (PostgreSQL), this takes a minute");
        }
        pg.setup().await.context("could not set up the built-in database")?;
        if pg.status() == Status::Started {
            // Left running by a previous run that was killed: restart it cleanly on our port.
            let _ = pg.stop().await;
        }
        pg.start().await.context("could not start the built-in database")?;
        if !pg.database_exists(DATABASE).await? {
            pg.create_database(DATABASE).await?;
        }
        let url = pg.settings().url(DATABASE);
        tracing::info!(port = pg.settings().port, data = %root.display(), "built-in database started");
        Ok((EmbeddedDb { pg }, url))
    }

    pub async fn stop(self) {
        if let Err(e) = self.pg.stop().await {
            tracing::warn!(error = %e, "could not stop the built-in database cleanly");
        }
    }
}

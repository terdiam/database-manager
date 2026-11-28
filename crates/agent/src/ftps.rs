//! Uploads over FTPS. OpenDAL closes the data connection without reading what the server
//! sent on it (TLS 1.3 session tickets), so the connection is reset and large files are
//! cut off. Everything else about FTPS still goes through OpenDAL.

use std::{sync::Arc, time::Duration};

use suppaftp::{
    tokio::{AsyncDataStream, AsyncRustlsConnector, AsyncRustlsFtpStream, AsyncRustlsStream},
    tokio_rustls::{
        TlsConnector,
        rustls::{ClientConfig, RootCertStore},
    },
    types::FileType,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::Result;

pub struct Login {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl Login {
    async fn connect(&self) -> Result<AsyncRustlsFtpStream> {
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = AsyncRustlsConnector::from(TlsConnector::from(Arc::new(config)));
        let mut ftp = AsyncRustlsFtpStream::connect(format!("{}:{}", self.host, self.port))
            .await
            .map_err(failed)?
            .into_secure(connector, &self.host)
            .await
            .map_err(failed)?;
        ftp.login(&self.username, &self.password)
            .await
            .map_err(failed)?;
        ftp.transfer_type(FileType::Binary).await.map_err(failed)?;
        Ok(ftp)
    }
}

fn failed(error: suppaftp::FtpError) -> String {
    format!("FTPS: {error}")
}

pub struct Upload {
    login: Arc<Login>,
    ftp: AsyncRustlsFtpStream,
    data: AsyncDataStream<AsyncRustlsStream>,
    part: String,
    path: String,
}

impl Upload {
    /// Writes to `<path>.part`; `folders` are made first, outermost first.
    pub async fn start(login: Arc<Login>, folders: &[String], path: String) -> Result<Self> {
        let mut ftp = login.connect().await?;
        for folder in folders {
            // Fails when it exists; a real problem shows when the file is created.
            let _ = ftp.mkdir(folder).await;
        }
        let part = format!("{path}.part");
        let data = ftp.put_with_stream(&part).await.map_err(failed)?;
        Ok(Self {
            login,
            ftp,
            data,
            part,
            path,
        })
    }

    pub async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.data
            .write_all(bytes)
            .await
            .map_err(|e| format!("FTPS upload stopped: {e}"))?;
        Ok(())
    }

    pub async fn commit(mut self) -> Result<()> {
        self.data.shutdown().await?;
        // Read until the server closes: closing with unread data would reset the connection.
        let mut rest = [0; 1024];
        let _ = tokio::time::timeout(Duration::from_secs(60), async {
            while matches!(self.data.read(&mut rest).await, Ok(n) if n > 0) {}
        })
        .await;
        drop(self.data);
        // The stream is closed already; this reads the server's verdict on the transfer.
        self.ftp
            .finalize_put_stream(tokio::io::sink())
            .await
            .map_err(failed)?;
        self.ftp
            .rename(&self.part, &self.path)
            .await
            .map_err(failed)?;
        let _ = self.ftp.quit().await;
        Ok(())
    }

    /// On a new connection: this one still waits for the end of the transfer.
    pub async fn abort(self) -> Result<()> {
        drop(self.data);
        drop(self.ftp);
        let mut ftp = self.login.connect().await?;
        ftp.rm(&self.part).await.map_err(failed)?;
        let _ = ftp.quit().await;
        Ok(())
    }
}

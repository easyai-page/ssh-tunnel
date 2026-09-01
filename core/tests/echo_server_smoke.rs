mod support;

use russh::client::{self, AuthResult, Handler};
use russh::keys::PublicKeyOrCertificate;
use std::sync::Arc;
use support::*;

struct SmokeHandler;

impl Handler for SmokeHandler {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[tokio::test]
async fn connect_and_password_auth() {
    let server = start_ssh_server(TestServerOpts {
        password: Some(TEST_PASSWORD),
        accept_keys: vec![],
    })
    .await;
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect(config, server.addr, SmokeHandler).await.unwrap();
    let result = handle.authenticate_password("u", TEST_PASSWORD).await.unwrap();
    assert!(matches!(result, AuthResult::Success));
    server.shutdown.shutdown("test done".into());
}

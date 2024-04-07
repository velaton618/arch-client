use std::pin::{ pin, Pin };
use std::future::Future;

use futures_util::future::{ select, Either };
use grammers_client::{ Client, Update };
use tokio::task;

pub type Res = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub struct ArchConfig {
    pub session_file: String,
    pub api_id: i32,
    pub api_hash: String,
    pub prompt: fn(&str) -> String,
    pub handle_update: fn(Client, Update) -> Pin<Box<dyn Future<Output = Res> + Send>>,
}

pub struct ArchClient {
    pub config: ArchConfig,
    pub client: Client,
    pub sign_out: bool,
}

impl ArchClient {
    pub async fn start(self) {
        let client_handle = self.client.clone();
        let network_handle = task::spawn(async move { self.client.run_until_disconnected().await });

        if self.sign_out {
            drop(client_handle.sign_out_disconnect().await);
        }

        loop {
            let update = {
                let exit = pin!(async { tokio::signal::ctrl_c().await });
                let upd = pin!(async { client_handle.next_update().await });

                match select(exit, upd).await {
                    Either::Left(_) => None,
                    Either::Right((u, _)) => Some(u),
                }
            };

            let update = match update {
                None | Some(Ok(None)) => {
                    break;
                }
                Some(u) => u.unwrap().unwrap(),
            };

            let handle = client_handle.clone();
            let handle_update = self.config.handle_update;

            task::spawn(async move {
                match handle_update(handle, update).await {
                    Ok(_) => {}
                    Err(e) => eprintln!("Error handling updates!: {}", e),
                }
            });
        }

        network_handle.await.unwrap().unwrap();
    }
}

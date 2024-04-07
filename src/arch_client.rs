use std::pin::{ pin, Pin };
use std::future::Future;

use futures_util::future::{ select, Either };
use grammers_client::{ Client, Config, SignInError, Update };
use grammers_session::Session;
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
    pub async fn new(cfg: ArchConfig) -> Self {
        log::info!("Connecting to Telegram...");

        let client = Client::connect(Config {
            session: Session::load_file_or_create(&cfg.session_file).unwrap(),
            api_id: cfg.api_id,
            api_hash: cfg.api_hash.clone(),
            params: Default::default(),
        }).await.unwrap();
        log::info!("Connected!");

        Self {
            client: client,
            sign_out: false,
            config: cfg,
        }
    }

    pub async fn sign_in(&mut self) {
        let mut sign_out = false;
        if !self.client.is_authorized().await.unwrap() {
            log::info!("Signing in...");

            let phone = (self.config.prompt)("Enter your phone number (international format): ");
            let token = self.client.request_login_code(&phone).await.unwrap();
            let code = (self.config.prompt)("Enter the code you received: ");

            let signed_in = self.client.sign_in(&token, &code).await;

            match signed_in {
                Err(SignInError::PasswordRequired(password_token)) => {
                    let hint = password_token.hint().unwrap_or("None");
                    let prompt_message = format!("Enter the password (hint {}): ", &hint);
                    let password = (self.config.prompt)(prompt_message.as_str());

                    self.client.check_password(password_token, password.trim()).await.unwrap();
                }
                Ok(_) => (),
                Err(e) => panic!("{}", e),
            }
            log::info!("Signed in!");

            match self.client.session().save_to_file(&self.config.session_file) {
                Ok(_) => {}
                Err(e) => {
                    log::error!("failed to save the session, will sign out when done: {}", e);
                    sign_out = true;
                }
            }
        }

        self.sign_out = sign_out;
    }

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

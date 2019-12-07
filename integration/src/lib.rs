#[cfg(test)]
mod tests {
    use commander::{commander_main, Opt};
    use executor::executor_main;
    use funtonic::config::Role::Commander;
    use funtonic::config::{CommanderConfig, Config, ExecutorConfig, Role, ServerConfig};
    use log::LevelFilter;
    use std::collections::BTreeMap;
    use std::time::Duration;
    use taskserver::taskserver_main;

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }

    #[tokio::test]
    async fn my_test() {
        let mut authorized_client_tokens = BTreeMap::new();
        authorized_client_tokens.insert("coucou".into(), "test client".into());
        let taskserver_config = Config {
            tls: None,
            role: Role::Server(ServerConfig {
                bind_address: "127.0.0.1:54010".to_string(),
                data_directory: "/tmp/taskserver".to_string(),
                authorized_client_tokens,
            }),
        };

        env_logger::builder().filter_level(LevelFilter::Info).init();

        std::thread::spawn(|| {
            let mut rt = tokio::runtime::Builder::new()
                .basic_scheduler()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(taskserver_main(taskserver_config)).unwrap();
        });

        let executor_config = Config {
            tls: None,
            role: Role::Executor(ExecutorConfig {
                client_id: "exec".to_string(),
                tags: Default::default(),
                server_url: "http://127.0.0.1:54010".to_string(),
            }),
        };
        std::thread::spawn(|| {
            let mut rt = tokio::runtime::Builder::new()
                .basic_scheduler()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(executor_main(executor_config)).unwrap();
        });

        let commander_config = Config {
            tls: None,
            role: Commander(CommanderConfig {
                server_url: "http://127.0.0.1:54010".to_string(),
                client_token: "coucou".to_string(),
            }),
        };

        let commander_opt = Opt {
            group: false,
            no_progress: false,
            config: None,
            query: "*".to_string(),
            command: vec!["cat".into(), "Cargo.toml".into()],
        };
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config)
            .await
            .expect("This must not throw errors");
    }
}

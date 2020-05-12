#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::{
        commander_config, executor_config, generate_valid_keys, run_cmd_opt, taskserver_config,
    };
    use commander::{commander_main, AdminCommand, AdminCommandOuputMode, Command, Opt};
    use executor::executor_main;
    use log::LevelFilter;
    use std::sync::Once;
    use std::time::Duration;
    use taskserver::taskserver_main;

    static INIT_LOGGER: Once = Once::new();

    fn init_logger() {
        INIT_LOGGER.call_once(|| env_logger::builder().filter_level(LevelFilter::Info).init())
    }

    #[tokio::test]
    async fn no_tls_test() {
        init_logger();

        let (priv_key, authorized_keys) = generate_valid_keys();

        let taskserver_config = taskserver_config(
            54010,
            false,
            authorized_keys.clone(),
            authorized_keys.clone(),
        );
        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));
        let executor_config = executor_config(54010, false, authorized_keys.clone());
        super::test_utils::spawn_future_on_new_thread(|| executor_main(executor_config));

        let commander_opt = run_cmd_opt("*", "cat Cargo.toml");
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config(54010, false, priv_key))
            .await
            .expect("cat Cargo.toml failed");
    }

    #[tokio::test]
    async fn tls_test() {
        init_logger();

        let (priv_key, authorized_keys) = generate_valid_keys();

        let taskserver_config = taskserver_config(
            54011,
            true,
            authorized_keys.clone(),
            authorized_keys.clone(),
        );
        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));

        let executor_config = executor_config(54011, true, authorized_keys.clone());
        super::test_utils::spawn_future_on_new_thread(|| executor_main(executor_config));

        let commander_opt = run_cmd_opt("*", "cat Cargo.toml");
        std::thread::sleep(Duration::from_secs(1));
        commander_main(
            commander_opt,
            commander_config(54011, true, priv_key.clone()),
        )
        .await
        .expect("cat Cargo.toml failed");

        let commander_opt = Opt {
            config: None,
            command: Command::Admin {
                output_mode: AdminCommandOuputMode::Json,
                command: AdminCommand::ListConnectedExecutors { query: None },
            },
        };
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config(54011, true, priv_key))
            .await
            .unwrap();
    }
}

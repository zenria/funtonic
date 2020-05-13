#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::{
        admin_cmd, commander_config, executor_config, run_cmd_opt, taskserver_config,
    };
    use commander::commander_main;
    use executor::executor_main;
    use funtonic::signed_payload::generate_base64_encoded_keys;
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

        let (priv_key, authorized_keys) = generate_base64_encoded_keys("tests");

        let (executor_private_key, _) = generate_base64_encoded_keys("local_executor");

        let taskserver_config = taskserver_config(
            54010,
            false,
            authorized_keys.clone(),
            authorized_keys.clone(),
        );
        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));
        let executor_config = executor_config(54010, false, authorized_keys.clone());
        super::test_utils::spawn_future_on_new_thread(|| {
            executor_main(executor_config, executor_private_key)
        });

        let commander_opt = run_cmd_opt("*", "cat Cargo.toml");
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config(54010, false, priv_key))
            .await
            .expect("cat Cargo.toml failed");
    }

    #[tokio::test]
    async fn tls_test() {
        init_logger();

        let (priv_key, authorized_keys) = generate_base64_encoded_keys("tests");
        let (executor_private_key, _) = generate_base64_encoded_keys("local_executor");

        let taskserver_config = taskserver_config(
            54011,
            true,
            authorized_keys.clone(),
            authorized_keys.clone(),
        );
        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));

        let executor_config = executor_config(54011, true, authorized_keys.clone());
        super::test_utils::spawn_future_on_new_thread(|| {
            executor_main(executor_config, executor_private_key)
        });

        std::thread::sleep(Duration::from_secs(1));

        // accessing the taskserver without tls is an error
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54011, false, priv_key.clone()),
        )
        .await
        .expect_err("Accessing tls server without tls must fail");

        // nominal case
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54011, true, priv_key.clone()),
        )
        .await
        .expect("cat Cargo.toml failed");

        commander_main(admin_cmd(), commander_config(54011, true, priv_key))
            .await
            .unwrap();

        // low level tls connection checks
        reqwest::get("https://127.0.0.1:54011")
            .await
            .expect_err("This must fail (unknown remote certificate, invalid host)");
        reqwest::ClientBuilder::new()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap()
            .get("https://127.0.0.1:54011")
            .send()
            .await
            .expect_err("Accepting invalid certificate still must fail (the server will not accept a connection without a specific certificate)");
    }

    #[tokio::test]
    async fn keys_test() {
        init_logger();
        // valid keys
        let (regular_key, mut authorized_keys) = generate_base64_encoded_keys("regular");
        let (admin_key, mut admin_authorized_keys) = generate_base64_encoded_keys("admin");
        // unknown or unauthorized keys
        let (unauthorized_regular_key, _) = generate_base64_encoded_keys("regular");
        let (unauthorized_unknown_key, _) = generate_base64_encoded_keys("unknown");
        let (unauthorized_admin_key, _) = generate_base64_encoded_keys("admin");

        // register an "ultimate" key both in normal & admin authorized key stores
        let (ultimate_key, ultimate_authorired_key) = generate_base64_encoded_keys("ultimate");

        let (executor_private_key, _) = generate_base64_encoded_keys("local_executor");

        authorized_keys.insert(
            "ultimate".into(),
            ultimate_authorired_key.get("ultimate").unwrap().clone(),
        );
        admin_authorized_keys.insert(
            "ultimate".into(),
            ultimate_authorired_key.get("ultimate").unwrap().clone(),
        );

        let taskserver_config =
            taskserver_config(54012, false, authorized_keys.clone(), admin_authorized_keys);
        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));
        let executor_config = executor_config(54012, false, authorized_keys.clone());
        super::test_utils::spawn_future_on_new_thread(|| {
            executor_main(executor_config, executor_private_key)
        });

        std::thread::sleep(Duration::from_secs(1));
        /////// ------------- Regular command forwarded to executors

        // executing a command with regular key must succeed
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54012, false, regular_key.clone()),
        )
        .await
        .expect("cat Cargo.toml failed");

        // executing a command with unauthorized keys must fail
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54012, false, unauthorized_regular_key.clone()),
        )
        .await
        .expect_err("Execution with unauth key must fail");
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54012, false, unauthorized_unknown_key.clone()),
        )
        .await
        .expect_err("Execution with unauth key must fail");
        // even with admin key, it must fail: the admin key is only registered on admin ops
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54012, false, admin_key.clone()),
        )
        .await
        .expect_err("Execution with unauth key must fail");

        /////// ------------- ADMIN

        commander_main(admin_cmd(), commander_config(54012, false, admin_key))
            .await
            .expect("This must not fail (admin command with admin key ;))");

        commander_main(admin_cmd(), commander_config(54012, false, regular_key))
            .await
            .expect_err("Non admin keys are not authorized");

        commander_main(
            admin_cmd(),
            commander_config(54012, false, unauthorized_regular_key),
        )
        .await
        .expect_err("Invalid non admin keys are not authorized");

        commander_main(
            admin_cmd(),
            commander_config(54012, false, unauthorized_admin_key),
        )
        .await
        .expect_err("Invalid admin keys are not authorized");

        commander_main(
            admin_cmd(),
            commander_config(54012, false, unauthorized_unknown_key),
        )
        .await
        .expect_err("unknown keys are not authorized");

        // check the ultimate key can do both regular cmd && admin cmd
        commander_main(
            admin_cmd(),
            commander_config(54012, false, ultimate_key.clone()),
        )
        .await
        .expect("This must not fail (admin command with admin key ;))");
        commander_main(
            run_cmd_opt("*", "cat Cargo.toml"),
            commander_config(54012, false, ultimate_key.clone()),
        )
        .await
        .expect("cat Cargo.toml failed");
    }
}

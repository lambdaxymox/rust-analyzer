use flexi_logger::{Duplicate, Logger};
use lsp_server::{run_server, stdio_transport, LspServerError};

use ra_lsp_server::{show_message, Result, ServerConfig};
use ra_prof;

fn main() -> Result<()> {
    std::env::set_var("RUST_BACKTRACE", "short");
    let logger = Logger::with_env_or_str("error").duplicate_to_stderr(Duplicate::All);
    match std::env::var("RA_LOG_DIR") {
        Ok(ref v) if v == "1" => logger.log_to_file().directory("log").start()?,
        _ => logger.start()?,
    };
    ra_prof::set_filter(match std::env::var("RA_PROFILE") {
        Ok(spec) => ra_prof::Filter::from_spec(&spec),
        Err(_) => ra_prof::Filter::disabled(),
    });
    log::info!("lifecycle: server started");
    match std::panic::catch_unwind(main_inner) {
        Ok(res) => {
            log::info!("lifecycle: terminating process with {:?}", res);
            res
        }
        Err(_) => {
            log::error!("server panicked");
            Err("server panicked")?
        }
    }
}

fn main_inner() -> Result<()> {
    let (sender, receiver, io_threads) = stdio_transport();
    let cwd = std::env::current_dir()?;
    let caps = serde_json::to_value(ra_lsp_server::server_capabilities()).unwrap();
    run_server(caps, sender, receiver, |params, s, r| {
        let params: lsp_types::InitializeParams = serde_json::from_value(params)?;
        let root = params.root_uri.and_then(|it| it.to_file_path().ok()).unwrap_or(cwd);

        let workspace_roots = params
            .workspace_folders
            .map(|workspaces| {
                workspaces
                    .into_iter()
                    .filter_map(|it| it.uri.to_file_path().ok())
                    .collect::<Vec<_>>()
            })
            .filter(|workspaces| !workspaces.is_empty())
            .unwrap_or_else(|| vec![root]);

        let server_config: ServerConfig = params
            .initialization_options
            .and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| {
                        log::error!("failed to deserialize config: {}", e);
                        show_message(
                            lsp_types::MessageType::Error,
                            format!("failed to deserialize config: {}", e),
                            s,
                        );
                    })
                    .ok()
            })
            .unwrap_or_default();

        ra_lsp_server::main_loop(workspace_roots, params.capabilities, server_config, r, s)
    })
    .map_err(|err| match err {
        LspServerError::ProtocolError(err) => err.into(),
        LspServerError::ServerError(err) => err,
    })?;
    log::info!("shutting down IO...");
    io_threads.join()?;
    log::info!("... IO is down");
    Ok(())
}

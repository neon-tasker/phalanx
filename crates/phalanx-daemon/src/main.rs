//! Phalanx Daemon: In-memory simulation proxy with sub-command execution support.
use alloy_primitives::{address, Bytes};
use phalanx_daemon::engine::{PipelinedJitDb, SimulationExecutor};
use phalanx_daemon::rpc::{build_router, AppState, CallObject};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;

#[derive(Debug)]
enum DaemonMode {
    Serve,
    Verify,
    Fork(Vec<String>),
}

#[derive(Debug)]
struct CliConfig {
    upstream_url: String,
    bind_addr: SocketAddr,
    mode: DaemonMode,
}

fn parse_cli_args() -> CliConfig {
    let mut upstream_url = std::env::var("PHALANX_UPSTREAM_RPC")
        .unwrap_or_else(|_| "https://eth.drpc.org".to_string());
    let mut bind_addr: SocketAddr = std::env::var("PHALANX_BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8545".to_string())
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 8545)));
    let mut mode = DaemonMode::Serve;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "verify" | "--verify" => {
                mode = DaemonMode::Verify;
            }
            "fork" => {
                let mut cmd_args = Vec::new();
                i += 1;
                if i < args.len() && args[i] == "--" {
                    i += 1;
                }
                while i < args.len() {
                    cmd_args.push(args[i].clone());
                    i += 1;
                }
                if cmd_args.is_empty() {
                    eprintln!("Error: 'fork' requires a command payload after '--'.");
                    eprintln!("Example: phalanx-daemon fork -- forge test");
                    std::process::exit(1);
                }
                mode = DaemonMode::Fork(cmd_args);
                break;
            }
            "--upstream" => {
                if i + 1 < args.len() {
                    upstream_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--bind" => {
                if i + 1 < args.len() {
                    if let Ok(addr) = args[i + 1].parse() {
                        bind_addr = addr;
                    }
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!("Phalanx Daemon v0.2.0 - Low-Latency In-Memory revm Simulation Engine\n");
                println!("USAGE:");
                println!("  phalanx-daemon [OPTIONS] [COMMAND]\n");
                println!("COMMANDS:");
                println!("  verify                     Run live 5-sector benchmark audit against upstream");
                println!("  fork -- <COMMAND...>       Spawn background proxy, inject ETH_RPC_URL, run command\n");
                println!("OPTIONS:");
                println!("  --upstream <URL>           Upstream Ethereum RPC [default: https://eth.drpc.org]");
                println!("  --bind <ADDR>              Proxy socket address  [default: 127.0.0.1:8545]");
                println!("  --help, -h                 Print help information");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    CliConfig {
        upstream_url,
        bind_addr,
        mode,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "phalanx_daemon=info,tower_http=warn".into()),
        )
        .init();

    let config = parse_cli_args();

    let http_client = reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(32)
        .tcp_nodelay(true)
        .build()?;

    let jit_db = Arc::new(PipelinedJitDb::new(
        config.upstream_url.clone(),
        http_client.clone(),
    ));
    let executor = Arc::new(SimulationExecutor::new(jit_db));

    match config.mode {
        DaemonMode::Verify => {
            run_live_verification_audit(&executor, &config.upstream_url).await;
            Ok(())
        }
        DaemonMode::Fork(child_cmd) => {
            run_fork_wrapper(config.upstream_url, config.bind_addr, executor, http_client, child_cmd).await
        }
        DaemonMode::Serve => {
            println!("================================================================================");
            println!("             PHALANX DAEMON: IN-MEMORY SPECULATIVE SIMULATION PROXY            ");
            println!("================================================================================");
            println!("Target Upstream RPC: {}", config.upstream_url);
            println!("Listening Socket:    http://{}", config.bind_addr);

            let state = AppState {
                upstream_url: config.upstream_url,
                http_client,
                executor,
            };

            let router = build_router(state);
            let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
            info!("Phalanx Proxy active on http://{}", config.bind_addr);
            axum::serve(listener, router).await?;
            Ok(())
        }
    }
}

async fn run_fork_wrapper(
    upstream_url: String,
    bind_addr: SocketAddr,
    executor: Arc<SimulationExecutor>,
    http_client: reqwest::Client,
    child_cmd: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        upstream_url,
        http_client: http_client.clone(),
        executor,
    };

    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let rpc_endpoint = format!("http://{}", bind_addr);

    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let health_url = format!("{}/health", rpc_endpoint);
    let mut ready = false;
    for _ in 0..50 {
        if let Ok(res) = http_client.get(&health_url).send().await {
            if res.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    if !ready {
        eprintln!("[ERROR] Phalanx proxy failed to start on {}", rpc_endpoint);
        server_handle.abort();
        std::process::exit(1);
    }

    println!("==> Phalanx Daemon background proxy listening on {}", rpc_endpoint);
    println!("==> Spawning target command: {}", child_cmd.join(" "));

    let mut cmd = tokio::process::Command::new(&child_cmd[0]);
    if child_cmd.len() > 1 {
        cmd.args(&child_cmd[1..]);
    }
    cmd.env("ETH_RPC_URL", &rpc_endpoint);

    let mut child = cmd.spawn().map_err(|e| {
        server_handle.abort();
        format!("Failed to spawn child command '{}': {e}", child_cmd[0])
    })?;

    let exit_status = child.wait().await?;
    server_handle.abort();

    let code = exit_status.code().unwrap_or(1);
    std::process::exit(code);
}

async fn run_live_verification_audit(executor: &SimulationExecutor, upstream_url: &str) {
    println!("\nExecuting Live Verification Audit against: {}", upstream_url);
    println!("Testing across 5 core Web3 sectors...\n");

    let caller_eoa = address!("0000000000000000000000000000000000000001");

    let targets = [
        (
            "Sector 1",
            "Auditor Fuzz (USDT Supply)",
            address!("dac17f958d2ee523a2206206994597c13d831ec7"),
            Bytes::from_static(&[0x18, 0x16, 0x0d, 0xdd]),
        ),
        (
            "Sector 2",
            "MEV / Solver (UniV3 slot0)",
            address!("88e6a0c2ddd26feeb64f039a2c41296fcb3f5640"),
            Bytes::from_static(&[0x38, 0x50, 0xc7, 0xbd]),
        ),
        (
            "Sector 3",
            "Wallet Guard (WETH Balance)",
            address!("c02aaa39b223fe8d0a0e5c4f27eed9083c756cc2"),
            Bytes::from_static(&[
                0x70, 0xa0, 0x82, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0xd8, 0xda, 0x6b, 0xf2, 0x69, 0x64, 0xaf, 0x9d, 0x7e, 0xed,
                0x9e, 0x03, 0xe5, 0x34, 0x15, 0xd3, 0x7a, 0xa9, 0x60, 0x45,
            ]),
        ),
        (
            "Sector 4",
            "Indexing (DAI decimals)",
            address!("6b175474e89094c44da98b954eedeac495271d0f"),
            Bytes::from_static(&[0x31, 0x3c, 0xe5, 0x67]),
        ),
    ];

    println!("+----------+-----------------------------+-----------+-----------+----------+-----------+");
    println!("| Sector   | Domain / Workload           | Cold (ms) | Warm (ms) | Speedup  | Status    |");
    println!("+----------+-----------------------------+-----------+-----------+----------+-----------+");

    for (sector, label, to_addr, calldata) in targets {
        let call = CallObject {
            from: Some(caller_eoa),
            to: Some(to_addr),
            data: Some(calldata),
            ..Default::default()
        };

        let t0 = Instant::now();
        let cold_res = executor
            .eth_call(Some(json!([call.clone(), "latest"])))
            .await;
        let cold_ms = t0.elapsed().as_nanos() as f64 / 1_000_000.0;

        let mut warm_durations = Vec::with_capacity(9);
        let mut success = cold_res.is_ok();

        if success {
            for _ in 0..9 {
                let tw = Instant::now();
                if executor
                    .eth_call(Some(json!([call.clone(), "latest"])))
                    .await
                    .is_err()
                {
                    success = false;
                    break;
                }
                warm_durations.push(tw.elapsed().as_nanos() as f64 / 1_000_000.0);
            }
        }

        let warm_ms = if !warm_durations.is_empty() {
            warm_durations.iter().sum::<f64>() / (warm_durations.len() as f64)
        } else {
            0.0
        };

        let speedup = if warm_ms > 0.0 {
            cold_ms / warm_ms
        } else {
            0.0
        };
        let status = if success && warm_ms < 1.0 {
            "CERTIFIED"
        } else if success {
            "PASS"
        } else {
            "UPSTREAM ERR"
        };

        println!(
            "| {:8} | {:27} | {:7.2}ms | {:7.3}ms | {:7.1}x | {:9} |",
            sector, label, cold_ms, warm_ms, speedup, status
        );
    }

    println!("+----------+-----------------------------+-----------+-----------+----------+-----------+");
    println!("Pre-flight verification finished.\n");
}
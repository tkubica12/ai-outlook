#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let account = match args.as_slice() {
        [] => None,
        [arg, account] if arg == "--sign-in" => Some(account.as_str()),
        _ => {
            eprintln!("Usage: workiq-hosted-probe [--sign-in <expected-work-email>]");
            std::process::exit(1);
        }
    };
    match tomlook::workiq::hosted::probe(account).await {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(text) => {
                println!("{text}");
                if !report.authenticated_read_verified {
                    std::process::exit(2);
                }
            }
            Err(error) => {
                eprintln!("Serialize hosted WorkIQ evidence: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("Hosted WorkIQ experiment: {error}");
            std::process::exit(1);
        }
    }
}

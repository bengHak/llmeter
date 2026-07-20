#[tokio::main]
async fn main() {
    let code = match llmeter::cli::run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("llmeter: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

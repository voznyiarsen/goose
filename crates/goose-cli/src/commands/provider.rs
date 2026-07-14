use anyhow::Result;
use console::style;
use goose::config::Config;

pub async fn handle_provider(provider_name: Option<String>) -> Result<()> {
    let config = Config::global();

    let provider_name = match provider_name {
        Some(name) => name,
        None => {
            let current = config.get_goose_provider().ok();
            match current {
                Some(name) => {
                    println!("Current provider: '{}'", name);
                    return Ok(());
                }
                None => {
                    println!(
                        "{}",
                        style("No provider configured. Run `goose configure` first.").yellow()
                    );
                    return Ok(());
                }
            }
        }
    };

    config.set_goose_provider(&provider_name)?;
    println!(
        "{} Switched to provider '{}'",
        console::style("✓").green(),
        provider_name
    );
    Ok(())
}

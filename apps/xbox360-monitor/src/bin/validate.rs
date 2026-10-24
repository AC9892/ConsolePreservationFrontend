fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("CPF_XBOX360_VALIDATE_PATH")
        .map_err(|_| "set CPF_XBOX360_VALIDATE_PATH to an explicitly selected local backup")?;
    let mode = std::env::var("CPF_XBOX360_VALIDATE_MODE").unwrap_or_else(|_| "automatic".into());
    let summary = cpf_xbox360::validate_source_with_mode(&path, &mode)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

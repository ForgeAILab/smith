fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = std::fs::read(&args[1])?;
    let output = if args[1].ends_with(".wat") {
        wat::parse_bytes(&input)?.into_owned()
    } else {
        wit_component::ComponentEncoder::default()
            .module(&input)?
            .validate(true)
            .encode()?
    };
    std::fs::write(&args[2], output)?;
    Ok(())
}

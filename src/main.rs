mod events;
mod mcp;
mod pet;

use std::io::{self, BufRead as _, Write as _};

use mcp::McpServer;
use pet::PetClient;

fn main() -> io::Result<()> {
    let pet = PetClient::from_environment()?;
    let mut server = McpServer::new(pet);
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            serde_json::to_writer(&mut stdout, &response).map_err(io::Error::other)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

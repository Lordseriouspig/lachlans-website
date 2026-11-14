use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn main() {
    let address = SocketAddr::from(([127, 0, 0, 1], 4000));
    let result =
        TcpStream::connect_timeout(&address, Duration::from_secs(2)).and_then(|mut stream| {
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.write_all(
                b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )?;

            let mut status = String::new();
            BufReader::new(stream).read_line(&mut status)?;
            if status.starts_with("HTTP/1.1 200 ") || status.starts_with("HTTP/1.0 200 ") {
                Ok(())
            } else {
                Err(std::io::Error::other(format!(
                    "unexpected health response: {status}"
                )))
            }
        });

    if let Err(error) = result {
        eprintln!("backend health check failed: {error}");
        std::process::exit(1);
    }
}

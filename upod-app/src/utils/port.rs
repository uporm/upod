use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};

use uuid::Uuid;

const MIN_PORT: u16 = 40_000;
const MAX_PORT: u16 = 60_000;
const MAX_ATTEMPTS: usize = 50;

pub(crate) fn find_random_available_port() -> io::Result<u16> {
    for _ in 0..MAX_ATTEMPTS {
        let port = random_port_in_range();
        if is_port_available(port) {
            return Ok(port);
        }
    }

    Err(io::Error::new(
        ErrorKind::AddrNotAvailable,
        format!(
            "failed to find an available port in range {MIN_PORT}-{MAX_PORT} after {MAX_ATTEMPTS} attempts"
        ),
    ))
}

fn random_port_in_range() -> u16 {
    let span = u32::from(MAX_PORT - MIN_PORT) + 1;
    let value = Uuid::new_v4().as_u128() % u128::from(span);
    MIN_PORT + value as u16
}

fn is_port_available(port: u16) -> bool {
    if is_bound_by_in_use((Ipv4Addr::LOCALHOST, port)) {
        return false;
    }
    if is_bound_by_in_use((Ipv6Addr::LOCALHOST, port)) {
        return false;
    }
    true
}

fn is_bound_by_in_use<T>(address: T) -> bool
where
    T: std::net::ToSocketAddrs,
{
    match TcpListener::bind(address) {
        Ok(listener) => {
            drop(listener);
            false
        }
        Err(error) => error.kind() == ErrorKind::AddrInUse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_port_in_range_is_bounded() {
        for _ in 0..200 {
            let port = random_port_in_range();
            assert!((MIN_PORT..=MAX_PORT).contains(&port));
        }
    }

    #[test]
    fn find_random_available_port_is_bounded() {
        let port = find_random_available_port().expect("should find available port");
        assert!((MIN_PORT..=MAX_PORT).contains(&port));
    }

    #[test]
    fn is_port_available_returns_false_when_port_is_occupied() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .expect("bind test port");
        let port = listener.local_addr().expect("read local addr").port();
        assert!(!is_port_available(port));
    }
}

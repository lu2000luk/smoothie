use std::{
    io,
    net::TcpListener,
    sync::atomic::{AtomicUsize, Ordering},
};

const RANGE_START: u16 = 5000;
const RANGE_END: u16 = 65000;

static NEXT_OFFSET: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct PortReservation {
    listener: TcpListener,
}

impl PortReservation {
    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .expect("a bound TCP listener always has a local address")
            .port()
    }

    pub fn into_listener(self) -> TcpListener {
        self.listener
    }
}

pub fn reserve_free_port() -> io::Result<PortReservation> {
    reserve_free_port_in_range(RANGE_START, RANGE_END)
}

pub fn reserve_free_port_in_range(start: u16, end: u16) -> io::Result<PortReservation> {
    if start > end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "port range start must not exceed its end",
        ));
    }

    let range_len = usize::from(end) - usize::from(start) + 1;
    let first_offset = NEXT_OFFSET.fetch_add(1, Ordering::Relaxed) % range_len;

    for offset in 0..range_len {
        let port = start + ((first_offset + offset) % range_len) as u16;
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(PortReservation { listener }),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AddrNotAvailable,
        format!("no free TCP port in range {start}..={end}"),
    ))
}

pub fn find_free_port() -> io::Result<u16> {
    let reservation = reserve_free_port()?;
    Ok(reservation.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_a_port_in_the_requested_range() {
        let reservation: PortReservation = reserve_free_port_in_range(55000, 55010).unwrap();
        assert!((55000..=55010).contains(&reservation.port()));
    }

    #[test]
    fn rejects_an_inverted_range() {
        let error = reserve_free_port_in_range(2, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

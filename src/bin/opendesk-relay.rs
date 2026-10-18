use std::{
    net::TcpListener,
    sync::{Arc, atomic::AtomicBool},
};
fn main() -> std::io::Result<()> {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7879".into());
    let listener = TcpListener::bind(&address)?;
    println!("Opendesk relay listening on {}", listener.local_addr()?);
    opendesk::relay::serve(listener, Arc::new(AtomicBool::new(false)))
}

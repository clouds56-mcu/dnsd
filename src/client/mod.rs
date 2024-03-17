use hickory_server::resolver::name_server::TokioConnectionProvider;

pub mod vanilla;
pub mod window;
pub mod udp;

pub use window::TimeWindowUdpProvider;

pub type Vanilla<P = TokioConnectionProvider> = vanilla::Client<P>;
pub type WindowedUdpClient = vanilla::Client<TimeWindowUdpProvider>;

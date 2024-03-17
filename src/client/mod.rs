use hickory_server::resolver::name_server::{GenericConnector, TokioConnectionProvider};

pub use self::memory_rt::MemoryRuntime;

pub mod vanilla;
pub mod window;
pub mod udp;
pub mod udp_stream;
pub mod memory_rt;

pub use window::TimeWindowUdpProvider;

pub type Vanilla<P = TokioConnectionProvider> = vanilla::Client<P>;
pub type MemoryClient = vanilla::Client<GenericConnector<MemoryRuntime>>;
pub type WindowedUdpClient = vanilla::Client<TimeWindowUdpProvider>;

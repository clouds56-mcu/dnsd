use hickory_server::resolver::name_server::{GenericConnector, TokioConnectionProvider};

pub use self::memory_rt::MemoryRuntime;

pub mod vanilla;
pub mod memory_rt;

pub type VanillaClient<P = TokioConnectionProvider> = vanilla::Client<P>;
pub type MemoryClient = vanilla::Client<GenericConnector<MemoryRuntime>>;

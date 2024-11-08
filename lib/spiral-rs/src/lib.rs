env::set_var("RAYON_NUM_THREADS", "1");

pub mod aligned_memory;
pub mod arith;
pub mod discrete_gaussian;
pub mod noise_estimate;
pub mod number_theory;
pub mod util;

pub mod gadget;
pub mod ntt;
pub mod params;
pub mod poly;

pub mod client;
pub mod key_value;

#[cfg(feature = "server")]
pub mod server;


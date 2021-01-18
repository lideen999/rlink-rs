#[macro_use]
extern crate log;
#[macro_use]
extern crate rlink_derive;

mod buffer_gen;
mod job;

pub fn main() {
    rlink::api::env::execute("showcase", crate::job::join::MyStreamJob {});
}
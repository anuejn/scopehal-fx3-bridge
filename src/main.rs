mod fx3_programmer;
mod fx3lafw;

use std::error::Error;
use fx3lafw::{acquisition, setup_device};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let device = setup_device()?;
    acquisition(&device, 48, 16)?;

    Ok(())
}

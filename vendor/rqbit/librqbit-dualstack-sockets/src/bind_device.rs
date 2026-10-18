#[cfg(test)]
pub(crate) mod tests;

use crate::Error;
use std::{num::NonZeroU32, str::FromStr};

#[derive(Debug, Clone)]
pub struct BindDevice {
    binding: rqbit_runtime::net::InterfaceBinding,
}

impl BindDevice {
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        rqbit_runtime::net::InterfaceBinding::new(name)
            .map(|binding| Self { binding })
            .map_err(Error::BindDeviceInvalidError)
    }

    pub fn index(&self) -> NonZeroU32 {
        self.binding.index()
    }

    pub fn name(&self) -> &str {
        self.binding.name()
    }

    pub fn binding(&self) -> &rqbit_runtime::net::InterfaceBinding {
        &self.binding
    }
    pub fn check_ready(&self) -> std::io::Result<()> {
        self.binding.check_ready()
    }
    pub fn bind_sref(&self, sref: &socket2::Socket, _is_v6: bool) -> crate::Result<()> {
        self.binding
            .bind_socket(sref)
            .map_err(Error::BindDeviceSetDeviceError)
    }
}

impl FromStr for BindDevice {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new_from_name(s)
    }
}

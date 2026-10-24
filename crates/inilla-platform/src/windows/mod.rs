//! Vanillish: **the Windows host** — the Win32 and WinRT calls behind `inilla-platform`'s plain
//! functions, one module per concern. Everything here is `cfg(windows)`: the Linux gates see it
//! only through the cross check (`windows/toolchain/linux-env.sh`).

pub(crate) mod fs;
pub(crate) mod net;

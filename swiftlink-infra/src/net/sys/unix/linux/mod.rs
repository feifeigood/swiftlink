fn set_bindtodevice<S: AsRawFd>(socket: &S, iface: &str) -> io::Result<()> {
    let iface_bytes = iface.as_bytes();

    unsafe {
        let ret = libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            iface_bytes.as_ptr() as *const _ as *const libc::c_void,
            iface_bytes.len() as libc::socklen_t,
        );

        if ret != 0 {
            let err = io::Error::last_os_error();
            error!("set SO_BINDTODEVICE error: {}", err);
            return Err(err);
        }
    }

    Ok(())
}

cfg_if! {
    if #[cfg(target_os = "android")] {
        use std::{
            io::ErrorKind,
            path::Path,
        };
        use tokio::io::AsyncReadExt;

        use super::uds::UnixStream;

        /// This is a RPC for Android to `protect()` socket for connecting to remote servers
        ///
        /// https://developer.android.com/reference/android/net/VpnService#protect(java.net.Socket)
        ///
        /// More detail could be found in [shadowsocks-android](https://github.com/shadowsocks/shadowsocks-android) project.
        async fn vpn_protect<P: AsRef<Path>>(protect_path: P, fd: RawFd) -> io::Result<()> {
            let mut stream = UnixStream::connect(protect_path).await?;

            // send fds
            let dummy: [u8; 1] = [1];
            let fds: [RawFd; 1] = [fd];
            stream.send_with_fd(&dummy, &fds).await?;

            // receive the return value
            let mut response = [0; 1];
            stream.read_exact(&mut response).await?;

            if response[0] == 0xFF {
                return Err(io::Error::new(ErrorKind::Other, "protect() failed"));
            }

            Ok(())
        }
    }
}

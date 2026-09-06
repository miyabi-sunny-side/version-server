use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_version-server"));
    command
        .env_remove("PORT")
        .env("APP_BIND_ADDR", "invalid-legacy-address");
    command
        .env("VERSION_SERVER_DB", ":memory:")
        .env("WATCH_REPOS", "");
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    command
}

#[test]
fn serves_selected_port_and_ignores_legacy_address() {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let mut server = Server(
        command()
            .env("PORT", address.port().to_string())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "server exited before listening"
        );
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            break stream;
        }
        assert!(Instant::now() < deadline, "server did not listen on PORT");
        thread::sleep(Duration::from_millis(20));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"GET /api/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#"{"status":"ok"}"#), "{response}");
}

fn assert_rejected(value: &std::ffi::OsStr) {
    let mut server = Server(command().env("PORT", value).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "invalid PORT must fail startup");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    let mut error = String::new();
    server
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut error)
        .unwrap();
    assert!(error.contains("PORT"), "{error}");
}

#[test]
fn invalid_port_fails_startup() {
    for raw in ["", "0", "65536", "bad", "127.0.0.1:3000"] {
        assert_rejected(raw.as_ref());
    }
}

#[cfg(unix)]
#[test]
fn non_unicode_port_fails_startup() {
    use std::os::unix::ffi::OsStrExt;
    assert_rejected(std::ffi::OsStr::from_bytes(&[0xff]));
}

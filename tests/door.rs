//! The door to the Internet: what a device reaches on a Host with a public
//! address, and the certificate it is reached under.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use orbite::Door;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A port of this machine nothing listens on yet.
fn free_port() -> SocketAddr {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    taken.local_addr().unwrap()
}

/// The interface behind the door, answering "the interface" at `/`.
async fn interface() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("the interface"))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_device_reaches_the_interface_encrypted_and_is_sent_there_from_the_plain_port() {
    let home = tempfile::tempdir().unwrap();
    let behind = interface().await;
    let (plain, encrypted) = (free_port(), free_port());
    // A certificate already given, for this address.
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    std::fs::create_dir_all(home.path().join("door")).unwrap();
    std::fs::write(home.path().join("door/cert.pem"), made.cert.pem()).unwrap();
    std::fs::write(home.path().join("door/key.pem"), made.signing_key.serialize_pem()).unwrap();
    std::fs::write(home.path().join("door/address"), "127.0.0.1").unwrap();
    let address: IpAddr = "127.0.0.1".parse().unwrap();
    tokio::spawn(
        // No authority is ever asked here: the certificate is young.
        Door::of(home.path(), address, *behind.address())
            .asking("http://127.0.0.1:9/directory", None)
            .listening_on(plain, encrypted)
            .open(),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;

    let trusting = reqwest::Certificate::from_pem(made.cert.pem().as_bytes()).unwrap();
    let device = reqwest::Client::builder()
        .add_root_certificate(trusting)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let page = device.get(format!("https://127.0.0.1:{}/", encrypted.port())).send().await.unwrap();
    assert_eq!(page.text().await.unwrap(), "the interface");

    // On the port that is not encrypted, only the way to the one that is.
    let sent =
        device.get(format!("http://127.0.0.1:{}/anything", plain.port())).send().await.unwrap();
    assert_eq!(sent.status(), 301);
    assert_eq!(
        sent.headers()["location"].to_str().unwrap(),
        format!("https://127.0.0.1:{}/", encrypted.port())
    );
    // And nothing of the interface is served there.
    assert_eq!(behind.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn one_address_holds_only_so_many_connections_and_the_others_are_still_served() {
    let home = tempfile::tempdir().unwrap();
    let behind = interface().await;
    let (plain, encrypted) = (free_port(), free_port());
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    std::fs::create_dir_all(home.path().join("door")).unwrap();
    std::fs::write(home.path().join("door/cert.pem"), made.cert.pem()).unwrap();
    std::fs::write(home.path().join("door/key.pem"), made.signing_key.serialize_pem()).unwrap();
    std::fs::write(home.path().join("door/address"), "127.0.0.1").unwrap();
    tokio::spawn(
        Door::of(home.path(), "127.0.0.1".parse().unwrap(), *behind.address())
            .asking("http://127.0.0.1:9/directory", None)
            .listening_on(plain, encrypted)
            .open(),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Connections that say nothing, as many as one address may hold.
    let mut silent = Vec::new();
    for _ in 0..32 {
        silent.push(tokio::net::TcpStream::connect(encrypted).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    // One more from there is closed at once, rather than kept.
    use tokio::io::AsyncReadExt;
    let mut more = tokio::net::TcpStream::connect(encrypted).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(3), more.read(&mut [0u8; 8])).await;
    assert!(matches!(closed, Ok(Ok(0)) | Ok(Err(_))), "it was kept: {closed:?}");
    // Once those give up their places, the address is served again.
    drop(silent);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let trusting = reqwest::Certificate::from_pem(made.cert.pem().as_bytes()).unwrap();
    let device = reqwest::Client::builder().add_root_certificate(trusting).build().unwrap();
    let page = device.get(format!("https://127.0.0.1:{}/", encrypted.port())).send().await.unwrap();
    assert_eq!(page.text().await.unwrap(), "the interface");
}

/// Run against a certificate authority of one's own (Let's Encrypt's
/// Pebble): `ORBITE_TEST_ACME_DIRECTORY`, the certificate it is trusted by
/// in `ORBITE_TEST_ACME_ROOT`, and the port it proves addresses on in
/// `ORBITE_TEST_ACME_PROVES_ON`.
#[tokio::test]
#[ignore = "needs a certificate authority to ask"]
async fn a_certificate_for_the_address_is_asked_proved_and_served_without_a_word_from_anyone() {
    let directory = std::env::var("ORBITE_TEST_ACME_DIRECTORY").unwrap();
    let root = std::env::var("ORBITE_TEST_ACME_ROOT").unwrap();
    let proves_on: u16 = std::env::var("ORBITE_TEST_ACME_PROVES_ON").unwrap().parse().unwrap();
    let home = tempfile::tempdir().unwrap();
    let behind = interface().await;
    let encrypted = free_port();
    let address: IpAddr = "127.0.0.1".parse().unwrap();
    tokio::spawn(
        Door::of(home.path(), address, *behind.address())
            .asking(directory, Some(root.into()))
            .listening_on(SocketAddr::new(address, proves_on), encrypted)
            .open(),
    );

    // Given within a minute, and kept for the Agent's account alone.
    let given = home.path().join("door/cert.pem");
    for _ in 0..120 {
        if given.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let trouble = std::fs::read_to_string(home.path().join("door/trouble")).unwrap_or_default();
    assert!(given.exists(), "no certificate was given: {trouble}");
    use std::os::unix::fs::PermissionsExt;
    let key = std::fs::metadata(home.path().join("door/key.pem")).unwrap();
    assert_eq!(key.permissions().mode() & 0o777, 0o600);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // A device that trusts the authority reaches the interface under it.
    let chain = std::fs::read(&given).unwrap();
    let issuers: Vec<reqwest::Certificate> = reqwest::Certificate::from_pem_bundle(&chain).unwrap();
    let mut device = reqwest::Client::builder();
    for issuer in issuers.into_iter().skip(1) {
        device = device.add_root_certificate(issuer);
    }
    let page = device
        .build()
        .unwrap()
        .get(format!("https://127.0.0.1:{}/", encrypted.port()))
        .send()
        .await
        .unwrap();
    assert_eq!(page.text().await.unwrap(), "the interface");
}

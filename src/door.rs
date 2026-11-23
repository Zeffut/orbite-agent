//! The door to the Internet: on a Host with a public address, the web
//! interface is reached at that address, encrypted, under a certificate
//! Let's Encrypt gives to the address itself. No name to buy, nothing to
//! configure: the certificate is asked for, proved on port 80, and asked
//! for again every few days, for as long as Orbite runs.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A certificate for an address lasts under a week: it is asked for again
/// once it is this old, which leaves days to try again when that fails.
const ASKED_AGAIN_AFTER: Duration = Duration::from_secs(60 * 3600);

/// How many devices may be connected at once: more are turned away rather
/// than left to take everything the Host has.
const AT_ONCE: usize = 1024;

/// And how many of those one address may hold.
const FROM_ONE_ADDRESS: usize = 32;

/// A connection nothing passes through for this long is closed: the
/// interface itself says something to a device every few seconds.
const IDLE: Duration = Duration::from_secs(120);

/// How many connections each address holds.
#[derive(Default)]
struct Visitors(Mutex<HashMap<IpAddr, usize>>);

/// One connection of an address, counted until it ends.
struct Visit {
    among: Arc<Visitors>,
    from: IpAddr,
}

impl Visitors {
    /// Counts one more connection from `from`, unless it holds `most`.
    fn admit(self: &Arc<Self>, from: IpAddr, most: usize) -> Option<Visit> {
        let mut held = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = held.entry(from).or_default();
        if *count >= most {
            return None;
        }
        *count += 1;
        Some(Visit { among: self.clone(), from })
    }
}

impl Drop for Visit {
    fn drop(&mut self) {
        let mut held = self.among.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = held.get_mut(&self.from) {
            *count -= 1;
            if *count == 0 {
                held.remove(&self.from);
            }
        }
    }
}

/// The next connection to `listening`, when there is room for it and for
/// the address it comes from.
async fn admitted(
    listening: &TcpListener,
    at_once: &Arc<tokio::sync::Semaphore>,
    visitors: &Arc<Visitors>,
    from_one: usize,
) -> Option<(TcpStream, tokio::sync::OwnedSemaphorePermit, Visit)> {
    let (stream, from) = match listening.accept().await {
        Ok(accepted) => accepted,
        // Out of something the system gives: waited for, not spun on.
        Err(_) => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            return None;
        }
    };
    let place = at_once.clone().try_acquire_owned().ok()?;
    let visit = visitors.admit(from.ip(), from_one)?;
    Some((stream, place, visit))
}

/// What Let's Encrypt is told to prove the address is ours, by its token.
type Proofs = Arc<Mutex<HashMap<String, String>>>;

/// How the encrypted side answers, once there is a certificate.
type Encrypting = Arc<RwLock<Option<Arc<rustls::ServerConfig>>>>;

pub struct Door {
    home: PathBuf,
    address: IpAddr,
    behind: SocketAddr,
    directory: String,
    trusting: Option<PathBuf>,
    proving_on: SocketAddr,
    serving_on: SocketAddr,
}

impl Door {
    /// The door of the installation in `home`, whose public address is
    /// `address`, to the interface served at `behind`.
    pub fn of(home: impl Into<PathBuf>, address: IpAddr, behind: SocketAddr) -> Self {
        let everyone: IpAddr = match address {
            IpAddr::V4(_) => std::net::Ipv4Addr::UNSPECIFIED.into(),
            IpAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
        };
        Self {
            home: home.into(),
            address,
            behind,
            directory: LetsEncrypt::Production.url().to_owned(),
            trusting: None,
            proving_on: SocketAddr::new(everyone, 80),
            serving_on: SocketAddr::new(everyone, 443),
        }
    }

    /// Asks the certificate of the authority whose directory is at
    /// `directory`, itself trusted by the certificate in `trusting`.
    pub fn asking(mut self, directory: impl Into<String>, trusting: Option<PathBuf>) -> Self {
        self.directory = directory.into();
        self.trusting = trusting;
        self
    }

    /// Proves the address at `proving_on` and serves at `serving_on`,
    /// rather than on the ports the Internet expects.
    pub fn listening_on(mut self, proving_on: SocketAddr, serving_on: SocketAddr) -> Self {
        self.proving_on = proving_on;
        self.serving_on = serving_on;
        self
    }

    /// Where what the door keeps is: the account at the authority, the
    /// certificate and its key, and what went wrong last.
    fn kept(&self) -> PathBuf {
        self.home.join("door")
    }

    /// Opens the door and keeps it open: this ends only when it could not
    /// be opened at all.
    pub async fn open(self) -> io::Result<()> {
        let proving = TcpListener::bind(self.proving_on).await?;
        let serving = TcpListener::bind(self.serving_on).await?;
        {
            use std::os::unix::fs::DirBuilderExt;
            let made = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(self.kept());
            made?;
        }
        let proofs: Proofs = Arc::default();
        let encrypting: Encrypting = Arc::default();
        if self.is_fresh()
            && let Ok(config) = encrypting_with(&self.kept())
        {
            *encrypting.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(config);
        }
        tokio::spawn(prove(proving, proofs.clone(), self.address, self.serving_on.port()));
        tokio::spawn(serve(serving, encrypting.clone(), self.behind));
        // The certificate, now and every time it grows old; a failure is
        // written down for the Owner and tried again, further and further
        // apart, as the authority only lets so many attempts through.
        let mut waits = Duration::from_secs(15 * 60);
        loop {
            if !self.is_fresh() {
                match self.certificate(&proofs).await {
                    Ok(()) => {
                        let _ = std::fs::remove_file(self.kept().join("trouble"));
                        if let Ok(config) = encrypting_with(&self.kept()) {
                            *encrypting.write().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                Some(config);
                        }
                        waits = Duration::from_secs(15 * 60);
                    }
                    Err(trouble) => {
                        eprintln!("The certificate for {} was not given: {trouble}", self.address);
                        let _ = std::fs::write(self.kept().join("trouble"), &trouble);
                        tokio::time::sleep(waits).await;
                        waits = (waits * 2).min(Duration::from_secs(6 * 3600));
                        continue;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }

    /// True when there is a certificate young enough to keep.
    fn is_fresh(&self) -> bool {
        // Given to another address, it is of no use at this one.
        let for_this = std::fs::read_to_string(self.kept().join("address"))
            .is_ok_and(|given_to| given_to.trim() == self.address.to_string());
        let given =
            std::fs::metadata(self.kept().join("cert.pem")).and_then(|kept| kept.modified());
        for_this
            && given.is_ok_and(|given| {
                SystemTime::now().duration_since(given).is_ok_and(|age| age < ASKED_AGAIN_AFTER)
            })
    }

    /// Asks the authority for a certificate for the address, proving the
    /// address is this Host's by answering on it, and keeps it.
    async fn certificate(&self, proofs: &Proofs) -> Result<(), String> {
        let said = |error: instant_acme::Error| error.to_string();
        let account = self.account().await?;
        let identifiers = [Identifier::Ip(self.address)];
        let mut asked = NewOrder::new(&identifiers);
        // Certificates for addresses are the short-lived kind, where the
        // authority has kinds.
        if account.profiles().any(|profile| profile.name == "shortlived") {
            asked = asked.profile("shortlived");
        }
        let mut order = account.new_order(&asked).await.map_err(said)?;
        let mut authorizations = order.authorizations();
        while let Some(authorization) = authorizations.next().await {
            let mut authorization = authorization.map_err(said)?;
            if authorization.status != AuthorizationStatus::Pending {
                continue;
            }
            let mut challenge = authorization
                .challenge(ChallengeType::Http01)
                .ok_or("the authority offers no proof this Host can give")?;
            let proof = challenge.key_authorization().as_str().to_owned();
            proofs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(challenge.token.clone(), proof);
            challenge.set_ready().await.map_err(said)?;
        }
        let patience = RetryPolicy::default();
        let ready = order.poll_ready(&patience).await.map_err(said);
        proofs.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
        if ready? != OrderStatus::Ready {
            return Err(format!(
                "the authority could not reach this Host at {} on port 80: that port and port \
                 443 must be open to the Internet",
                self.address
            ));
        }
        let key = order.finalize().await.map_err(said)?;
        let chain = order.poll_certificate(&patience).await.map_err(said)?;
        // The key first, and readable by the Agent's account alone.
        let kept = self.kept();
        write_private(&kept.join("key.pem"), key.as_bytes()).map_err(|error| error.to_string())?;
        write_private(&kept.join("cert.pem"), chain.as_bytes())
            .and_then(|()| std::fs::write(kept.join("address"), self.address.to_string()))
            .map_err(|error| error.to_string())
    }

    /// This installation's account at the authority: the one it has, or a
    /// new one, which asks nothing of the Owner.
    async fn account(&self) -> Result<Account, String> {
        let said = |error: instant_acme::Error| error.to_string();
        let builder = || match &self.trusting {
            Some(root) => Account::builder_with_root(root),
            None => Account::builder(),
        };
        let kept = self.kept().join("account.json");
        let known = std::fs::read(&kept)
            .ok()
            .and_then(|kept| serde_json::from_slice::<AccountCredentials>(&kept).ok());
        if let Some(credentials) = known {
            return builder().map_err(said)?.from_credentials(credentials).await.map_err(said);
        }
        let new =
            NewAccount { contact: &[], terms_of_service_agreed: true, only_return_existing: false };
        let (account, credentials) = builder()
            .map_err(said)?
            .create(&new, self.directory.clone(), None)
            .await
            .map_err(said)?;
        let written = serde_json::to_vec(&credentials).map_err(|error| error.to_string())?;
        write_private(&kept, &written).map_err(|error| error.to_string())?;
        Ok(account)
    }
}

/// Writes `content` to `file`, for the Agent's account alone to read.
fn write_private(file: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::remove_file(file);
    let mut written =
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file)?;
    written.write_all(content)
}

/// How to encrypt with the certificate and key kept in `kept`.
fn encrypting_with(kept: &Path) -> Result<Arc<rustls::ServerConfig>, String> {
    let said = |error: rustls::pki_types::pem::Error| error.to_string();
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(kept.join("cert.pem"))
        .map_err(said)?
        .collect::<Result<_, _>>()
        .map_err(said)?;
    let key = PrivateKeyDer::from_pem_file(kept.join("key.pem")).map_err(said)?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|error| error.to_string())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Answers on the port that is not encrypted: what the authority asks to
/// prove the address, and for anything else, the way to the encrypted side.
async fn prove(listening: TcpListener, proofs: Proofs, address: IpAddr, encrypted_port: u16) {
    let at_once = Arc::new(tokio::sync::Semaphore::new(AT_ONCE));
    let visitors = Arc::<Visitors>::default();
    loop {
        // Few from one address: the authority asks from several of its own.
        let Some((mut asking, place, visit)) = admitted(&listening, &at_once, &visitors, 8).await
        else {
            continue;
        };
        let proofs = proofs.clone();
        tokio::spawn(async move {
            let _held = (place, visit);
            let answer = async {
                // The first line says what is asked; nothing more is read.
                let mut head = vec![0u8; 2048];
                let mut read = 0;
                while read < head.len() && !head[..read].contains(&b'\n') {
                    match asking.read(&mut head[read..]).await {
                        Ok(more) if more > 0 => read += more,
                        _ => break,
                    }
                }
                let first = String::from_utf8_lossy(&head[..read]);
                let path = first.split_whitespace().nth(1).unwrap_or_default();
                let proof = path.strip_prefix("/.well-known/acme-challenge/").and_then(|token| {
                    proofs
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .get(token)
                        .cloned()
                });
                let answer = match proof {
                    Some(proof) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{proof}",
                        proof.len()
                    ),
                    None => {
                        let host = match address {
                            IpAddr::V4(address) => address.to_string(),
                            IpAddr::V6(address) => format!("[{address}]"),
                        };
                        let port = if encrypted_port == 443 {
                            String::new()
                        } else {
                            format!(":{encrypted_port}")
                        };
                        format!(
                            "HTTP/1.1 301 Moved Permanently\r\nLocation: https://{host}{port}/\r\n\
                             Content-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                    }
                };
                let _ = asking.write_all(answer.as_bytes()).await;
                let _ = asking.shutdown().await;
            };
            let _ = tokio::time::timeout(Duration::from_secs(10), answer).await;
        });
    }
}

/// Serves the encrypted side: what a device says is decrypted and passed
/// to the interface behind, and its answers encrypted back.
async fn serve(listening: TcpListener, encrypting: Encrypting, behind: SocketAddr) {
    let at_once = Arc::new(tokio::sync::Semaphore::new(AT_ONCE));
    let visitors = Arc::<Visitors>::default();
    loop {
        let Some((device, place, visit)) =
            admitted(&listening, &at_once, &visitors, FROM_ONE_ADDRESS).await
        else {
            continue;
        };
        // Until there is a certificate, nobody is answered.
        let Some(config) =
            encrypting.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
        else {
            continue;
        };
        tokio::spawn(async move {
            let _held = (place, visit);
            let greeted = tokio_rustls::TlsAcceptor::from(config).accept(device);
            let Ok(Ok(device)) = tokio::time::timeout(Duration::from_secs(15), greeted).await
            else {
                return;
            };
            let Ok(mut interface) = TcpStream::connect(behind).await else { return };
            let (mut from_device, mut to_device) = tokio::io::split(device);
            let (mut from_interface, mut to_interface) = interface.split();
            let (mut said, mut answered) = (vec![0u8; 16 * 1024], vec![0u8; 16 * 1024]);
            // Passed on both ways, until one side ends or nothing passes
            // for too long.
            loop {
                let passing = async {
                    tokio::select! {
                        read = from_device.read(&mut said) => match read {
                            Ok(read) if read > 0 => to_interface.write_all(&said[..read]).await.is_ok(),
                            _ => false,
                        },
                        read = from_interface.read(&mut answered) => match read {
                            Ok(read) if read > 0 => to_device.write_all(&answered[..read]).await.is_ok(),
                            _ => false,
                        },
                    }
                };
                if tokio::time::timeout(IDLE, passing).await != Ok(true) {
                    break;
                }
            }
            let _ = to_device.shutdown().await;
        });
    }
}

/// True when `address` is one a device on the Internet could reach.
pub fn is_of_the_internet(address: IpAddr) -> bool {
    let special = match address {
        IpAddr::V4(address) => {
            address.is_unspecified() || address.is_broadcast() || address.is_multicast()
        }
        // One of the older kind written as the newer is given as the older.
        IpAddr::V6(address) => {
            address.is_unspecified() || address.is_multicast() || address.to_ipv4_mapped().is_some()
        }
    };
    !special && !crate::is_private(&SocketAddr::new(address, 0))
}

/// The address this Host has on the Internet, when one of its own network
/// cards carries it: none on a machine behind another's address.
pub fn public_address() -> Option<IpAddr> {
    let mut first: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: the list is the system's, read here and freed below.
    if unsafe { libc::getifaddrs(&mut first) } != 0 {
        return None;
    }
    let mut found = None;
    let mut card = first;
    while !card.is_null() {
        // SAFETY: `card` is an entry of the list the system gave.
        let entry = unsafe { &*card };
        if !entry.ifa_addr.is_null()
            && entry.ifa_flags & libc::IFF_UP as u32 != 0
            // SAFETY: the address is read as the kind its family says.
            && unsafe { (*entry.ifa_addr).sa_family } == libc::AF_INET as libc::sa_family_t
        {
            // SAFETY: an address of the Internet family has this layout.
            let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
            let address = std::net::Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr));
            let reached = !crate::is_private(&SocketAddr::new(address.into(), 0))
                && !address.is_unspecified()
                && !address.is_broadcast()
                && !address.is_documentation();
            if reached && found.is_none() {
                found = Some(IpAddr::V4(address));
            }
        }
        card = entry.ifa_next;
    }
    // SAFETY: the list given by `getifaddrs` above, freed once.
    unsafe { libc::freeifaddrs(first) };
    found
}

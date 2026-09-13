/// Cryptographic data types, like hashes and signatures.
///
/// # Unstable
///
/// These types may be split out into a separate crate in the future,
/// and have experimental APIs.
use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, Payload},
    AeadCore, ChaCha20Poly1305, Key, KeyInit, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::OsRng;
use snafu::Snafu;

use crate::{
    codec::{CodecError, Decodable, Encodable, FieldReader, Format, WritesEncodable},
    sized_byte_array,
    stream::Writes,
    types::binary::hex_from_bytes,
};

use super::Coda;

sized_byte_array!(
    /// Byte array containing a Blake3 hash.
    HashBytes,
    32
);

sized_byte_array!(
    /// Byte array containing an Ed25519 private key.
    PrivateKeyBytes,
    32
);

sized_byte_array!(
    /// Byte array containing an Ed25519 public key.
    PublicKeyBytes,
    32
);

sized_byte_array!(
    /// Byte array containing an Ed25519 signature.
    SignatureBytes,
    64
);

/// A hasher which creates [`HashBytes`].
#[derive(Default)]
pub struct CryptoHasher {
    hasher: blake3::Hasher,
}

impl CryptoHasher {
    /// Writes `bytes` to the in-progress hash.
    pub fn write(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    /// Completes the hash and consumes `self`, returning it as [HashBytes].
    pub fn finalize(self) -> HashBytes {
        HashBytes::from(*self.hasher.finalize().as_bytes())
    }

    /// Completes the hash and consumes `self`, writing it into `bytes`.
    pub fn finalize_into_bytes(self, bytes: &mut HashBytes) {
        bytes.0 = *self.hasher.finalize().as_bytes();
    }
}

impl Writes for CryptoHasher {
    fn write(&mut self, buf: &[u8]) -> Result<usize, crate::stream::StreamError> {
        self.hasher.update(buf);
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), crate::stream::StreamError> {
        self.hasher.update(buf);
        Ok(())
    }
}

/// Signing (private) and verifying (public)
/// key pair which can create and verify
/// [`SignatureBytes`].
pub struct CryptoKeys {
    signer: CryptoSigner,
    verifier: CryptoVerifier,
}

impl CryptoKeys {
    /// Generates and returns a new pair of keys.
    pub fn generate() -> Self {
        let mut rng = OsRng;
        let signer = SigningKey::generate(&mut rng);
        let verifier = signer.verifying_key();
        CryptoKeys {
            signer: CryptoSigner {
                private_key: signer,
            },
            verifier: CryptoVerifier {
                public_key: verifier,
            },
        }
    }

    /// Tries to load a pair of keys from
    /// `private_key`.
    pub fn from_private(private_key: PrivateKeyBytes) -> Result<Self, CryptoError> {
        let signer = SigningKey::from_bytes(&private_key.0);
        let verifier = signer.verifying_key();
        Ok(CryptoKeys {
            signer: CryptoSigner {
                private_key: signer,
            },
            verifier: CryptoVerifier {
                public_key: verifier,
            },
        })
    }

    /// Consumes these keys, returning _only_
    /// their private key.
    pub fn into_private(self) -> PrivateKeyBytes {
        let mut bytes = PrivateKeyBytes::default();
        let private_key = &self.signer.private_key.to_keypair_bytes()[0..PrivateKeyBytes::SIZE];
        bytes.copy_from_slice(private_key);
        bytes
    }
}

/// Signing (private) key which
/// creates [`SignatureBytes`].
pub struct CryptoSigner {
    private_key: SigningKey,
}

/// Verifying (public) key which
/// verifies [`SignatureBytes`].
#[derive(Copy, Clone, Debug)]
pub struct CryptoVerifier {
    public_key: VerifyingKey,
}

impl TryFrom<&PublicKeyBytes> for CryptoVerifier {
    type Error = CryptoError;

    fn try_from(public_key: &PublicKeyBytes) -> Result<Self, Self::Error> {
        let public_key =
            VerifyingKey::from_bytes(&public_key.0).map_err(|_| CryptoError::InvalidPublicKey {
                pub_key: *public_key,
            })?;

        Ok(CryptoVerifier { public_key })
    }
}

/// A cryptographic certificate, containing
/// [`SignatureBytes`] accompanied by the
/// [`PublicKeyBytes`] of the entity that
/// created the signature.
#[derive(Copy, Clone, Debug, Default)]
pub struct CryptoCert {
    /// The public key of the entity
    /// that created [`Self::signature`].
    pub public_key: PublicKeyBytes,

    /// The signature.
    pub signature: SignatureBytes,
}

impl CryptoCert {
    /// Signs `data` with `signer`, replacing
    /// `self`'s current signature with the result.
    pub fn sign(
        &mut self,
        signer: &impl CryptoSigns,
        data: &[&[u8]],
    ) -> core::result::Result<(), CryptoError> {
        self.public_key = signer.public_key_bytes();
        self.signature = signer.sign(data)?;

        Ok(())
    }

    /// Returns `Ok` iff this certificate's
    /// signature is valid and matches `data`.
    pub fn verify(&self, data: &[&[u8]]) -> core::result::Result<(), CryptoError> {
        let key = CryptoVerifier::try_from(&self.public_key)?;
        key.verify(data, &self.signature)
    }
}

impl Eq for CryptoCert {}
impl PartialEq for CryptoCert {
    fn eq(&self, other: &Self) -> bool {
        self.public_key == other.public_key && self.signature == other.signature
    }
}

impl Ord for CryptoCert {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.signature.cmp(&other.signature)
    }
}

impl PartialOrd for CryptoCert {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl core::hash::Hash for CryptoCert {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.public_key.hash(state);
        self.signature.hash(state);
    }
}

impl Encodable for CryptoCert {
    const FORMAT: Format = PublicKeyBytes::FORMAT.with(SignatureBytes::FORMAT);

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_data(&self.public_key)?;
        writer.write_data(&self.signature)?;
        Ok(())
    }
}

impl Decodable for CryptoCert {
    fn decode(
        &mut self,
        reader: &mut (impl crate::codec::ReadsDecodable + ?Sized),
        header: Option<crate::codec::DataHeader>,
    ) -> Result<(), CodecError> {
        Self::ensure_no_header(header)?;
        reader.read_data_into(&mut self.public_key)?;
        reader.read_data_into(&mut self.signature)?;
        Ok(())
    }
}

/// A thing that can be represented as a cryptographic hash.
pub trait HasCryptoHash {
    /// Writes `self`'s cryptographhically hashable
    /// data to `hasher`.
    fn crypto_hash_into(&self, hasher: &mut CryptoHasher);

    /// Returns a new [`CryptoHasher`] containing `self`'s
    /// cryptographically hashable data.
    ///
    /// The hashable data is not guaranteed to contain the
    /// entirety of `self`'s data. For example, some data
    /// structures may contain a certificate that indirectly
    /// contains all of `self`'s data; in these cases,
    /// the hashable data returned by this method may be
    /// the raw bytes of the certificate's signature.
    fn crypto_hasher(&self) -> CryptoHasher {
        let mut hasher = CryptoHasher::default();
        self.crypto_hash_into(&mut hasher);
        hasher
    }
}

impl HasCryptoHash for CryptoCert {
    fn crypto_hash_into(&self, hasher: &mut CryptoHasher) {
        hasher.write(&self.signature);
    }
}

impl HasCryptoHash for Coda {
    fn crypto_hash_into(&self, hasher: &mut CryptoHasher) {
        let _ = self.encode(hasher);
    }
}

/// A thing that has associated [`PublicKeyBytes`].
pub trait HasCryptoPublicKey {
    /// Returns this thing's public key.
    fn public_key_bytes(&self) -> PublicKeyBytes;
}

impl HasCryptoPublicKey for CryptoKeys {
    fn public_key_bytes(&self) -> PublicKeyBytes {
        self.verifier.public_key_bytes()
    }
}

impl HasCryptoPublicKey for CryptoSigner {
    fn public_key_bytes(&self) -> PublicKeyBytes {
        (*self.private_key.verifying_key().as_bytes()).into()
    }
}

impl HasCryptoPublicKey for CryptoVerifier {
    fn public_key_bytes(&self) -> PublicKeyBytes {
        (*self.public_key.as_bytes()).into()
    }
}

/// A thing that creates [`SignatureBytes`].
pub trait CryptoSigns: HasCryptoPublicKey {
    /// Signs `message` with this signer's private key,
    /// returning `Ok(signature)` iff signing was successful.
    fn sign(&self, message: &[&[u8]]) -> Result<SignatureBytes, CryptoError>;
}

impl CryptoSigns for CryptoKeys {
    fn sign(&self, message: &[&[u8]]) -> Result<SignatureBytes, CryptoError> {
        self.signer.sign(message)
    }
}

impl CryptoSigns for CryptoSigner {
    fn sign(&self, message: &[&[u8]]) -> Result<SignatureBytes, CryptoError> {
        let signature = self
            .private_key
            .try_sign(message.concat().as_slice())
            .expect("signing failure");
        Ok(signature.to_bytes().into())
    }
}

/// A thing that verifies [`SignatureBytes`].
pub trait CryptoVerifies: HasCryptoPublicKey {
    /// Verifies `signature` against `message`,
    /// returning `Ok` iff the `signature` is
    /// valid and corresponds to this verifier's
    /// public key.
    fn verify(&self, message: &[&[u8]], signature: &SignatureBytes) -> Result<(), CryptoError>;
}

impl CryptoVerifies for CryptoKeys {
    fn verify(&self, message: &[&[u8]], signature: &SignatureBytes) -> Result<(), CryptoError> {
        self.verifier.verify(message, signature)
    }
}

impl CryptoVerifies for CryptoVerifier {
    fn verify(&self, message: &[&[u8]], signature: &SignatureBytes) -> Result<(), CryptoError> {
        let message = message.concat();
        let message = message.as_slice();
        let sig = Signature::from_bytes(&signature.0);
        self.public_key
            .verify_strict(message, &sig)
            .map_err(|_| CryptoError::InvalidSignature {
                signature: *signature,
            })
    }
}

/// Data encrypted with `ChaCha20-Poly1305`
/// by a key derived via `Argon2`.
#[derive(Default)]
pub struct EncryptedData {
    /// Nonce used during key derivation,
    /// encryption, and decryption.
    nonce: [u8; 12],

    /// Encrypted data.
    data: alloc::vec::Vec<u8>,
}

impl EncryptedData {
    /// Encrypts `data` with `key`, returning a new encrypted data.
    pub fn new(key: &[u8], data: &[u8]) -> Result<Self, CryptoError> {
        // Generate a nonce for key derivation and encryption.
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

        // Derive key.
        let mut derived_key = [0u8; 32];
        Argon2::default().hash_password_into(key, nonce.as_ref(), &mut derived_key)?;
        let cipher = ChaCha20Poly1305::new(&Key::from(derived_key));

        // Encrypt the data, attaching the nonce as unencrypted additional associated data.
        let encrypted = cipher.encrypt(
            &nonce,
            Payload {
                msg: data,
                aad: &nonce,
            },
        )?;

        Ok(Self {
            nonce: nonce.into(),
            data: encrypted,
        })
    }

    /// Decrypts this data with `key`, returning
    /// the decrypted data.
    pub fn decrypt(&self, key: &[u8]) -> Result<alloc::vec::Vec<u8>, CryptoError> {
        // Derive key.
        let mut derived_key = [0u8; 32];
        Argon2::default().hash_password_into(key, self.nonce.as_ref(), &mut derived_key)?;
        let cipher = ChaCha20Poly1305::new(&Key::from(derived_key));

        // Decrypt the data.
        let decrypted = cipher.decrypt(
            Nonce::from_slice(&self.nonce),
            Payload {
                msg: &self.data,
                aad: &self.nonce,
            },
        )?;

        Ok(decrypted)
    }

    /// Returns a string containing the nonce and
    /// encrypted data in HEX format, separated by
    /// a `-` character.
    pub fn to_hex(&self) -> alloc::string::String {
        alloc::format!(
            "{}-{}",
            hex_from_bytes(&self.nonce),
            hex_from_bytes(&self.data)
        )
    }

    /// Returns a new encrypted data by decoding a
    /// string containing a `nonce-data` pair, where
    /// the `nonce` and `data` are HEX-encoded.
    pub fn from_hex(hex: &str) -> Result<Self, CryptoError> {
        let (nonce, key) = hex.split_once('-').ok_or(CryptoError::Malformed)?;
        let nonce = super::binary::bytes_from_hex(nonce).map_err(|_| CryptoError::Malformed)?;
        let key = super::binary::bytes_from_hex(key).map_err(|_| CryptoError::Malformed)?;

        Ok(EncryptedData {
            nonce: nonce.try_into().map_err(|_| CryptoError::Malformed)?,
            data: key,
        })
    }
}

impl Encodable for EncryptedData {
    const FORMAT: Format = Format::data(0)
        .with(<[u8; 12]>::FORMAT)
        .with(alloc::vec::Vec::<u8>::FORMAT);

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_data(&self.nonce)?;
        writer.write_data(&self.data)?;
        Ok(())
    }
}

impl Decodable for EncryptedData {
    fn decode(
        &mut self,
        reader: &mut (impl crate::codec::ReadsDecodable + ?Sized),
        header: Option<crate::codec::DataHeader>,
    ) -> Result<(), CodecError> {
        let header = Self::ensure_header(header)?;
        let mut fields = FieldReader::new(reader, header);
        fields.read(&mut self.nonce)?;
        fields.read(&mut self.data)?;
        fields.finish()
    }
}

/// An error that may occur when interacting with cryptographic data.
#[derive(Debug, Snafu, Clone)]
pub enum CryptoError {
    #[snafu(display("the private key could not be loaded as an Ed25519 private key"))]
    InvalidPrivateKey,

    #[snafu(display("{pub_key} could not be loaded as an Ed25519 public key"))]
    InvalidPublicKey { pub_key: PublicKeyBytes },

    #[snafu(display("{signature} was not a valid Ed25519 signature for the provided message"))]
    InvalidSignature { signature: SignatureBytes },

    #[snafu(display("deriving a cryptographic key failed: {message}"))]
    KeyDerivationFailure { message: alloc::string::String },

    #[snafu(display("encrypting or decrypting data failed: {message}"))]
    CipherFailure { message: alloc::string::String },

    #[snafu(display("the provided input was malformed or corrupt"))]
    Malformed,
}

impl From<argon2::Error> for CryptoError {
    fn from(value: argon2::Error) -> Self {
        Self::KeyDerivationFailure {
            message: <argon2::Error as crate::alloc::string::ToString>::to_string(&value),
        }
    }
}

impl From<chacha20poly1305::Error> for CryptoError {
    fn from(value: chacha20poly1305::Error) -> Self {
        Self::CipherFailure {
            message: <chacha20poly1305::Error as crate::alloc::string::ToString>::to_string(&value),
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::codec::ReadsDecodable;

    use super::*;

    #[test]
    fn encrypted_data() {
        let key = b"cupc4k3s";
        let message = b"i'm so secret.";

        // Test encryption happy-path.
        let mut encrypted = EncryptedData::new(key, message).unwrap();
        let decrypted = encrypted.decrypt(key).unwrap();
        assert_eq!(message, decrypted.as_slice());

        // Test that different encryptions use different nonces.
        let mut encrypted_too = EncryptedData::new(key, message).unwrap();
        assert_ne!(encrypted_too.data, encrypted.data);
        assert_ne!(encrypted_too.nonce, encrypted.nonce);

        // Test that mutating the nonce breaks decryption.
        encrypted.nonce.fill(0u8);
        assert!(encrypted.decrypt(key).is_err());

        // Test that mutating the payload breaks decryption.
        encrypted_too.data.fill(0u8);
        assert!(encrypted_too.decrypt(key).is_err());
    }

    #[test]
    fn encrypted_data_codas_codec() {
        let key = b"p4nc4k3s";
        let message = b"i'm pretty secret.";

        // Encrypt a message.
        let encrypted = EncryptedData::new(key, message).unwrap();

        // Encode the message payload.
        let mut encoded = vec![];
        encoded.write_data(&encrypted).unwrap();

        // Decode the message payload.
        let decoded: EncryptedData = encoded.as_slice().read_data().unwrap();

        // Test that the decoded data is still well-formatted.
        let decrypted = decoded.decrypt(key).unwrap();
        assert_eq!(message, decrypted.as_slice());
    }

    #[test]
    fn encrypted_data_hex_codec() {
        let key = b"p4nc4k3s";
        let message = b"i'm pretty secret.";

        // Encrypt a message.
        let encrypted = EncryptedData::new(key, message).unwrap();

        // Convert the message to hexadecimal.
        let encoded = encrypted.to_hex();

        let mut bytes = vec![];
        bytes.write_data(&encrypted).unwrap();
        eprintln!("raw hex: {encoded}");
        eprintln!("\n\ncoda hx: {}", hex_from_bytes(&bytes));

        // Decode the message contents.
        let decoded = EncryptedData::from_hex(&encoded).unwrap();
        assert_eq!(encrypted.nonce, decoded.nonce);
        assert_eq!(encrypted.data, decoded.data);
    }
}

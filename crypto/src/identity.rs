use pgp::{ser::Serialize, SignedPublicKey};
// Generate a cryptographic identity, using a nullifier and GPG public key
use super::{hash, CryptoHasherSha256};
/// A unique cryptographic identity for this voting system

pub type Nullifier = Vec<u8>;
pub type Identity = Vec<u8>;

pub struct UniqueIdentity {
    pub nullifier: Option<Nullifier>,
    pub identity: Option<Identity>,
}
impl UniqueIdentity {
    /// Function to generate the nullifier
    /// this must be random or else the nullifier
    /// can be cracked
    /// the consequence would be that
    ///     a) attacker can vote in your name
    ///     b) attacker can link your identity to a vote
    pub fn generate_nullifier(&mut self, random_seed: String) {
        // avoid accidental re-computation
        assert!(self.nullifier.is_none());
        self.nullifier = Some(hash(CryptoHasherSha256, &random_seed.as_bytes()));
    }
    pub fn compute_public_identity(&mut self, public_key: SignedPublicKey, vote: String) {
        assert!(self.nullifier.is_some() && self.identity.is_none());
        let mut payload = self.nullifier.clone().unwrap();
        payload.append(&mut public_key.to_bytes().unwrap());
        payload.append(vote.as_bytes().to_vec().as_mut());
        self.identity = Some(hash(CryptoHasherSha256, &payload));
    }
}

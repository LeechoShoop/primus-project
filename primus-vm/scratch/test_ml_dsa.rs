use ml_dsa::ml_dsa_87;
use ml_dsa::traits::{Signer, Verifier};

fn main() {
    let mut rng = rand::thread_rng();
    let (pk, sk) = ml_dsa_87::generate_keypair(&mut rng);
    let message = b"hello world";
    let sig = sk.sign(message);
    let res = pk.verify(message, &sig);
    println!("Verified: {:?}", res);
}

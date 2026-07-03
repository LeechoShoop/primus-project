use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier, Signature};
use rand::rngs::OsRng;
use rand::RngCore;

fn bench_ed25519_proxy(c: &mut Criterion) {
    let mut rng = OsRng;
    
    // GROUP 1: keygen
    c.bench_function("ml_dsa_87_keygen", |b| {
        b.iter(|| {
            let mut bytes = [0u8; 32];
            rng.fill_bytes(&mut bytes);
            let _ = SigningKey::from_bytes(&bytes);
        })
    });

    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    let signing_key = SigningKey::from_bytes(&bytes);
    let verifying_key = signing_key.verifying_key();
    let message = [42u8; 32];
    let signature = signing_key.sign(&message);

    // GROUP 2: sign
    c.bench_function("ml_dsa_87_sign", |b| {
        b.iter(|| {
            let _ = signing_key.sign(black_box(&message));
        })
    });

    // GROUP 3: verify_valid
    c.bench_function("ml_dsa_87_verify_valid", |b| {
        b.iter(|| {
            let res = verifying_key.verify(black_box(&message), black_box(&signature));
            assert!(res.is_ok());
        })
    });

    // GROUP 4: verify_invalid
    let mut sig_bytes = signature.to_bytes();
    sig_bytes[0] ^= 0xFF;
    let corrupted_signature = Signature::from_bytes(&sig_bytes);
    
    c.bench_function("ml_dsa_87_verify_invalid", |b| {
        b.iter(|| {
            let res = verifying_key.verify(black_box(&message), black_box(&corrupted_signature));
            assert!(res.is_err());
        })
    });

    // GROUP 5: verify_wrong_key
    let mut other_bytes = [0u8; 32];
    rng.fill_bytes(&mut other_bytes);
    let other_key = SigningKey::from_bytes(&other_bytes).verifying_key();
    c.bench_function("ml_dsa_87_verify_wrong_key", |b| {
        b.iter(|| {
            let res = other_key.verify(black_box(&message), black_box(&signature));
            assert!(res.is_err());
        })
    });
}

criterion_group!(benches, bench_ed25519_proxy);
criterion_main!(benches);

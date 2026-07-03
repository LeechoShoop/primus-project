use ml_dsa::{MlDsa87, SigningKey};
use rand::thread_rng;

fn main() {
    let mut rng = thread_rng();
    let sk = SigningKey::<MlDsa87>::generate(&mut rng);
}

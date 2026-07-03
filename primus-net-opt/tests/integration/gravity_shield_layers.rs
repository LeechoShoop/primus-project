use primus_net_opt::gravity_shield::GravityShield;
use primus_types::atom::Atom;
use primus_types::reaction::SignedReaction;

#[tokio::test]
async fn layer1_rejects_random_bytes() {
    let shield = GravityShield::new();
    assert!(shield.filter_bytes(&[0xFF; 100]).is_err());
    assert_eq!(shield.drop_count(), 1);
}

#[tokio::test]
async fn layer3_rejects_negative_energy() {
    let mut rx = SignedReaction {
        sender: Atom::new_materialized(vec![1; 2592], primus_types::atom::Element::Hydrogen),
        receiver: Atom::new_receiver(vec![2; 2592]),
        reaction_hash: [0; 32],
        energy: -1.0,
        timestamp: 0,
        signature: vec![1; 4627],
        payload: primus_types::payload::Payload::Transfer { amount: 0 },
    };
    rx.reaction_hash = rx.compute_reaction_hash();

    let bytes = bincode::serialize(&rx).unwrap();
    let shield = GravityShield::new();
    assert!(shield.filter_bytes(&bytes).is_err());
    assert_eq!(shield.drop_count(), 1);
}

#[tokio::test]
async fn layer3_rejects_empty_pubkey() {
    let mut rx = SignedReaction {
        sender: Atom::new_materialized(vec![], primus_types::atom::Element::Hydrogen),
        receiver: Atom::new_receiver(vec![2; 2592]),
        reaction_hash: [0; 32],
        energy: 1.0,
        timestamp: 0,
        signature: vec![1; 4627],
        payload: primus_types::payload::Payload::Transfer { amount: 0 },
    };
    rx.reaction_hash = rx.compute_reaction_hash();

    let bytes = bincode::serialize(&rx).unwrap();
    let shield = GravityShield::new();
    assert!(shield.filter_bytes(&bytes).is_err());
    assert_eq!(shield.drop_count(), 1);
}

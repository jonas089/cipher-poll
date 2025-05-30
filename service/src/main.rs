// responsible for maintaining state
// accepts proof payloads (Routes)
// verifies proofs
mod constants;
pub mod gauth;
use axum::{
    extract::DefaultBodyLimit,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use crossterm::{execute, terminal::Clear};
use gauth::query_user_gpg_keys;
use reqwest::StatusCode;
use risc0_zkvm::Receipt;
use std::{collections::HashMap, env, io};
use std::{collections::HashSet, sync::Arc};
// registers voters / inserts new identities into the tree
// if the signature is valid
// if the account is unique
// if the public key corresponds to the associated github keys
// for the user
use client::types::IdentityPayload;
use colored::*;
use crypto::{
    gpg::GpgSigner,
    hash,
    identity::{Identity, Nullifier},
    CryptoHasherSha256,
};
use pgp::types::Mpi;
use risc0_prover::verifier::verify_vote;
use tokio::sync::Mutex;
use voting_tree::VotingTree;
use zk_associated::storage::TreeState;

type GitHubUser = String;

#[derive(Clone)]
struct InMemoryTreeState {
    tree_state: TreeState,
}
impl InMemoryTreeState {
    fn insert_nullifier(&mut self, identity: Identity) {
        let new_state = self.tree_state.insert_nullifier(identity);
        self.tree_state = new_state;
    }
    fn insert_used_nullifier(&mut self, nullifier: Nullifier) {
        self.tree_state.used_nullifiers.push(nullifier);
    }
    fn get(&self) -> TreeState {
        // must be mutable, will be inserted -> clone
        self.tree_state.clone()
    }
}

#[derive(Clone)]
struct InMemoryGitHubUserState {
    github_users: HashSet<GitHubUser>,
}
impl InMemoryGitHubUserState {
    fn insert(&mut self, user: String) {
        self.github_users.insert(user);
    }
    fn get(&self, user: &String) -> Option<&String> {
        match self.github_users.get(user) {
            Some(user) => Some(user),
            None => None,
        }
    }
}

#[derive(Clone)]
struct InMemoryVoteState {
    votes: HashMap<String, u64>,
}
impl InMemoryVoteState {
    fn insert(&mut self, vote: String) {
        if self.votes.contains_key(&vote) {
            self.votes.insert(vote.clone(), self.get(&vote) + 1);
        } else {
            self.votes.insert(vote, 1u64);
        }
    }
    fn get(&self, vote: &String) -> &u64 {
        self.votes
            .get(vote)
            .expect(format!("Failed to get votes for {}", vote).as_str())
    }
}

#[derive(Clone)]
struct ServiceState {
    github_users: InMemoryGitHubUserState,
    tree_state: InMemoryTreeState,
    votes: InMemoryVoteState,
}
impl ServiceState {
    // register a voter, takes a risc0 receipt as input (currently not prover-generic)
    // todo: check that the GPG key is actually in the list of the GitHub User's associated Keys
    // using the GitHub API
    async fn process_registration_request(
        &mut self,
        signature: Vec<Mpi>,
        data: Vec<u8>,
        public_key: String,
        identity: Identity,
        username: GitHubUser,
    ) {
        let mut signer = GpgSigner {
            secret_key_asc_path: None,
            public_key_asc_string: Some(public_key),
            signed_secret_key: None,
            signed_public_key: None,
        };
        signer.init_verifier();
        // verify that the key exists in the Username's Raw Key List
        let raw_gpg_keys: Vec<String> =
            query_user_gpg_keys(env::var("GITHUB_TOKEN").unwrap()).await;
        assert!(raw_gpg_keys.contains(&signer.public_key_asc_string.clone().unwrap()));
        assert!(signer.is_valid_signature(signature, &data));
        if self.github_users.get(&username).is_some() {
            panic!("{}: Duplicate Github User", "Receted".red().bold())
        };
        self.github_users.insert(username.clone());
        self.tree_state.insert_nullifier(identity);
        println!(
            "{}{}: Github@{}",
            "+++++++++ \n".yellow(),
            "Accepted".bold().green(),
            &username.bold().green()
        )
    }
}

fn default_tree_state() -> TreeState {
    let mut voting_tree: VotingTree = VotingTree {
        zero_node: hash(CryptoHasherSha256, &vec![0; 32]),
        zero_levels: Vec::new(),
        // size must equal tree depth
        filled: vec![vec![]; 5],
        root: None,
        index: 0,
        // the maximum amount of identities this tree can store
        // is 2^depth (depth:5 => max_identity_count:32)
        depth: 5,
    };
    voting_tree.calculate_zero_levels();

    TreeState {
        root_history: Vec::new(),
        used_nullifiers: Vec::new(),
        voting_tree,
        leafs: Vec::new(),
    }
}

// `&'static str` becomes a `200 OK` with `content-type: text/plain; charset=utf-8`
async fn ping() -> &'static str {
    "pong"
}

#[tokio::main]
async fn main() {
    let mut stdout = io::stdout();
    execute!(stdout, Clear(crossterm::terminal::ClearType::All)).unwrap();
    print!(
        "{}",
        r#"
 ██████╗██╗   ██╗██████╗ ██╗  ██╗███████╗██████╗     ██████╗  ██████╗ ██╗     ██╗     
██╔════╝╚██╗ ██╔╝██╔══██╗██║  ██║██╔════╝██╔══██╗    ██╔══██╗██╔═══██╗██║     ██║     
██║      ╚████╔╝ ██████╔╝███████║█████╗  ██████╔╝    ██████╔╝██║   ██║██║     ██║     
██║       ╚██╔╝  ██╔═══╝ ██╔══██║██╔══╝  ██╔══██╗    ██╔═══╝ ██║   ██║██║     ██║     
╚██████╗   ██║   ██║     ██║  ██║███████╗██║  ██║    ██║     ╚██████╔╝███████╗███████╗
 ╚═════╝   ╚═╝   ╚═╝     ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝    ╚═╝      ╚═════╝ ╚══════╝╚══════╝
"#
        .red()
    );
    println!(
        "{}",
        " by Jonas Pauli, Casper Association "
            .bold()
            .white()
            .bold()
            .on_red()
    );
    let tree_state: TreeState = default_tree_state();
    let service_state: ServiceState = ServiceState {
        github_users: InMemoryGitHubUserState {
            github_users: HashSet::new(),
        },
        tree_state: InMemoryTreeState { tree_state },
        votes: InMemoryVoteState {
            votes: HashMap::new(),
        },
    };
    let shared_state = Arc::new(Mutex::new(service_state));
    let app = Router::new()
        .route(
            "/ping",
            get({
                //let shared_state = Arc::clone(&shared_state);
                move || ping()
            }),
        )
        .route("/register", post(register))
        .route("/vote", post(vote))
        .layer(DefaultBodyLimit::max(10000000))
        .layer(Extension(shared_state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn register(
    Extension(state): Extension<Arc<Mutex<ServiceState>>>,
    Json(payload): Json<IdentityPayload>,
) -> impl IntoResponse {
    let mut deserialized_signature: Vec<Mpi> = Vec::new();
    for series in &payload.signature_serialized {
        deserialized_signature.push(Mpi::from_slice(series))
    }
    state
        .lock()
        .await
        .tree_state
        .get()
        .voting_tree
        .add_leaf(payload.identity.clone());
    state
        .lock()
        .await
        .process_registration_request(
            deserialized_signature,
            payload.data_serialized,
            payload.public_key_string,
            payload.identity.clone(),
            payload.username,
        )
        .await;
    let snapshot_serialized: Vec<u8> =
        serde_json::to_vec(&state.lock().await.tree_state.get().voting_tree)
            .expect("Failed to serialize snapshot");
    (StatusCode::OK, snapshot_serialized)
}

async fn vote(
    Extension(state): Extension<Arc<Mutex<ServiceState>>>,
    Json(payload): Json<Receipt>,
) -> impl IntoResponse {
    let current_state = state.lock().await.tree_state.get();
    let (vote, nullifier): (String, Vec<u8>) = verify_vote(
        payload,
        current_state.root_history,
        current_state.used_nullifiers,
    );
    state.lock().await.votes.insert(vote.clone());
    state
        .lock()
        .await
        .tree_state
        .insert_used_nullifier(nullifier);
    println!(
        "{}{}: Anonymous User voted for {}",
        "+++++++++ \n".yellow(),
        "Accepted".bold().green(),
        &vote.bold().red()
    );
    println!(
        "{}Current State of the Election: {:?}",
        "+++++++++ \n".yellow(),
        &state.lock().await.votes.votes
    );
    (
        StatusCode::OK,
        format!("Accepted: Anonymous User voted for {}", &vote),
    )
}

#[tokio::test]
async fn submit_zk_vote() {
    use crypto::identity::UniqueIdentity;
    #[cfg(not(feature = "groth16"))]
    use risc0_prover::prover::prove_default as prover;
    #[cfg(feature = "groth16")]
    use risc0_prover::prover::prove_groth16 as prover;
    use risc0_types::CircuitInputs;
    use risc0_zkvm::Receipt;
    use std::{collections::HashSet, fs, path::PathBuf};
    // initialize tree_state and service_state
    // process a registration request using the default keypair in ~/resources/test/
    // generate a vote proof
    // verify the vote proof and apply the vote to tree_state
    let tree_state: TreeState = default_tree_state();
    let mut service_state: ServiceState = ServiceState {
        github_users: InMemoryGitHubUserState {
            github_users: HashSet::new(),
        },
        tree_state: InMemoryTreeState {
            tree_state: tree_state,
        },
        votes: InMemoryVoteState {
            votes: HashMap::new(),
        },
    };
    let mut identity: UniqueIdentity = UniqueIdentity {
        identity: None,
        nullifier: None,
    };
    identity.generate_nullifier("Hello".to_string());

    let private_key_path_str = "/Users/chef/Desktop/cypher-poll/resources/test/key.sec.asc";
    let public_key_path_str = "/Users/chef/Desktop/cypher-poll/resources/test/key.asc";

    let public_key_string: String =
        fs::read_to_string(public_key_path_str).expect("Failed to read public key");

    let mut signer = GpgSigner {
        secret_key_asc_path: Some(PathBuf::from(private_key_path_str)),
        public_key_asc_string: Some(public_key_string.clone()),
        signed_secret_key: None,
        signed_public_key: None,
    };
    signer.init();
    let data: Vec<u8> = vec![0u8];
    let signature: Vec<Mpi> = signer.sign_bytes(&data);

    let mut serialized_signature: Vec<Vec<u8>> = Vec::new();
    for mpi in &signature {
        serialized_signature.push(mpi.as_bytes().to_vec())
    }

    let mut deserialized_signature: Vec<Mpi> = Vec::new();
    for series in &serialized_signature {
        deserialized_signature.push(Mpi::from_slice(series))
    }
    assert_eq!(&signature, &deserialized_signature);

    assert!(signer.is_valid_signature(signature.clone(), &data));
    identity.compute_public_identity(signer.signed_public_key.unwrap(), "Overlord".to_string());
    // register the voter
    service_state
        .process_registration_request(
            signature,
            data,
            public_key_string.clone(),
            identity.identity.clone().expect("Missing identity"),
            "jonas089".to_string(),
        )
        .await;
    // generate a proof -> redeem the nullifier
    let proof: Receipt = prover(CircuitInputs {
        root_history: service_state.tree_state.get().root_history,
        snapshot: service_state.tree_state.get().voting_tree,
        nullifier: identity.nullifier.clone().expect("Missing Nullifier"),
        vote: "Overlord".to_string(),
        public_key_string: public_key_string.clone(),
    });
    verify_vote(
        proof,
        service_state.tree_state.get().root_history,
        service_state.tree_state.get().used_nullifiers,
    );
}

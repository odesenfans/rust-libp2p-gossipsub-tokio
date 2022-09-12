// Copyright 2018 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

//! A basic chat application with logs demonstrating libp2p and the gossipsub protocol.
//!
//! Using two terminal windows, start two instances. Type a message in either terminal and hit return: the
//! message is sent and printed in the other terminal. Close with Ctrl-c.
//!
//! You can of course open more terminal windows and add more participants.
//! Dialing any of the other peers will propagate the new participant to all
//! chat members and everyone will receive all messages.
//!
//! In order to get the nodes to connect, take note of the listening addresses of the first
//! instance and start the second with one of the addresses as the first argument. In the first
//! terminal window, run:
//!
//! ```sh
//! cargo run --example gossipsub-chat
//! ```
//!
//! It will print the [`PeerId`] and the listening addresses, e.g. `Listening on
//! "/ip4/0.0.0.0/tcp/24915"`
//!
//! In the second terminal window, start a new instance of the example with:
//!
//! ```sh
//! cargo run --example gossipsub-chat -- /ip4/127.0.0.1/tcp/24915
//! ```
//!
//! The two nodes should then connect.

use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use env_logger::{Builder, Env};
use futures::{prelude::*};
use libp2p::{core::upgrade, gossipsub, identity, mplex, Multiaddr, noise, PeerId, swarm::SwarmEvent, tcp::TokioTcpTransport, Transport};
use libp2p::gossipsub::{
    GossipsubEvent, GossipsubMessage, IdentTopic as Topic, MessageAuthenticity, ValidationMode,
};
use libp2p::gossipsub::MessageId;
use libp2p::swarm::SwarmBuilder;
use libp2p_tcp::GenTcpConfig;
use tokio;
use tokio::io::AsyncBufReadExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    Builder::from_env(Env::default().default_filter_or("info")).init();

    // Create a random PeerId
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {:?}", local_peer_id);

    // Set up an encrypted TCP Transport over the Mplex and Yamux protocols
    let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&local_key)
        .expect("Signing libp2p-noise static DH keypair failed.");

    let transport = TokioTcpTransport::new(GenTcpConfig::default().nodelay(true))
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    // Create a Gossipsub topic
    let topic = Topic::new("test-net");

    // Create a Swarm to manage peers and events
    let mut swarm = {
        // To content-address message, we can take the hash of message and use it as an ID.
        let message_id_fn = |message: &GossipsubMessage| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            MessageId::from(s.finish().to_string())
        };

        // Set a custom gossipsub
        let gossipsub_config = gossipsub::GossipsubConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
            .validation_mode(ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
            .message_id_fn(message_id_fn) // content-address messages. No two messages of the
            // same content will be propagated.
            .build()
            .expect("Valid config");
        // build a gossipsub network behaviour
        let mut gossipsub: gossipsub::Gossipsub =
            gossipsub::Gossipsub::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
                .expect("Correct configuration");

        // subscribes to our topic
        gossipsub.subscribe(&topic).unwrap();

        // add an explicit peer if one was provided
        if let Some(explicit) = std::env::args().nth(2) {
            match explicit.parse() {
                Ok(id) => gossipsub.add_explicit_peer(&id),
                Err(err) => println!("Failed to parse explicit peer id: {:?}", err),
            }
        }

        // build the swarm

        SwarmBuilder::new(transport, gossipsub, local_peer_id)
            // We want the connection background tasks to be spawned
            // onto the tokio runtime.
            .executor(Box::new(|fut| {
                tokio::spawn(fut);
            }))
            .build()
    };

    // Listen on all interfaces and whatever port the OS assigns
    swarm
        .listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())
        .unwrap();

    // Reach out to another node if specified
    if let Some(to_dial) = std::env::args().nth(1) {
        let address: Multiaddr = to_dial.parse().expect("User to provide valid address.");
        match swarm.dial(address.clone()) {
            Ok(_) => println!("Dialed {:?}", address),
            Err(e) => println!("Dial {:?} failed: {:?}", address, e),
        };
    }

    // Read full lines from stdin
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    // Kick it off
    loop {
        tokio::select! {
            line = stdin.next_line() => {
                let line = line?.expect("stdin closed");
                swarm.behaviour_mut().publish(topic.clone(), line.as_bytes()).unwrap();
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(GossipsubEvent::Message {
                    propagation_source: peer_id,
                    message_id: id,
                    message,
                }) => println!(
                    "Got message: {} with id: {} from peer: {:?}",
                    String::from_utf8_lossy(&message.data),
                    id,
                    peer_id
                ),
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Listening on {:?}", address);
                }
                _ => {}
            }
        }
    }
}

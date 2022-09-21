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

use std::error::Error;

use env_logger::{Builder, Env};
use libp2p::{identity, Multiaddr, PeerId};
use libp2p::gossipsub::IdentTopic as Topic;
use libp2p::multiaddr::Protocol;
use tokio;
use tokio::io::AsyncBufReadExt;
use futures::StreamExt;

mod network;

async fn dial_bootstrap_peers(network_client: &mut network::P2PClient, peers: &Vec<Multiaddr>) {
    for peer_addr in peers.iter() {
        let mut addr = peer_addr.clone();
        let last_protocol = addr.pop();
        let peer_id = match last_protocol {
            Some(Protocol::P2p(hash)) => PeerId::from_multihash(hash).expect("valid hash"),
            _ => {
                println!("Bootstrap peer multiaddr must end with its peer ID (/p2p/<peer-id>.");
                continue;
            }
        };
        match network_client.dial(peer_id, addr).await {
            Ok(_) => println!("Successfully dialed bootstrap peer: {}", &peer_addr),
            Err(e) => println!("Failed to dial bootstrap peer {}: {}", peer_addr, e),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    Builder::from_env(Env::default().default_filter_or("info")).init();

    // Create a random PeerId
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {:?}", local_peer_id);

    let (mut network_client, mut network_events, network_event_loop) =
        network::new(local_key).await?;

    let p2p_event_loop_handle = tokio::spawn(network_event_loop.run());

    // Listen on all interfaces and whatever port the OS assigns
    network_client.start_listening("/ip4/0.0.0.0/tcp/0".parse().unwrap()).await.unwrap();

    // Create a Gossipsub topic
    let topic = Topic::new("test-net");
    network_client.subscribe(&topic).await.unwrap();

    // Reach out to another node if specified
    if let Some(to_dial) = std::env::args().nth(1) {
        let multiaddr: Multiaddr = to_dial.parse().expect("should be a valid multiaddr");
        dial_bootstrap_peers(&mut network_client, &vec![multiaddr]).await;
    }

    // Read full lines from stdin
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    // Kick it off
    loop {
        tokio::select! {
            line = stdin.next_line() => {
                let line = line?.expect("stdin closed");
                if line == "exit" {
                    break;
                }
                network_client.publish(&topic, line.as_bytes()).await.unwrap();
            },
            network_event = network_events.next() => {
                match network_event {
                    Some(network::Event::PubsubMessage {
                        propagation_source: peer_id,
                        message_id: id,
                        message}) => println!(
                    "Got message: {} with id: {} from peer: {:?}",
                    String::from_utf8_lossy(&message.data),
                    id,
                    peer_id
                ),
                    None => {
                        println!("Network event loop stopped");
                        break;
                    }
                }
            }
        }
    }

    // let connected_peers: Vec<PeerId> = swarm.connected_peers().map(|peer_id| peer_id.clone()).collect();
    // println!("Connected peers: {:?}", connected_peers);
    // for peer in connected_peers {
    //     println!("Disconnecting {}...", peer);
    //     swarm.disconnect_peer_id(peer).expect("should be able to disconnect peer");
    // }
    //
    // tokio::time::sleep(Duration::from_secs(3)).await;

    println!("Goodbye");

    Ok(())
}

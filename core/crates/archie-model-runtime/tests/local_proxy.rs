use archie_model_runtime::{InferenceClient,ProviderEndpoint};
use archie_types::Locality;
use std::{io::{Read,Write},net::TcpListener,process::{Command,Stdio},sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};
#[test]
#[ignore="invoked by the parent with isolated proxy environment"]
fn local_proxy_worker() {
    let endpoint=ProviderEndpoint::openai_compatible(&std::env::var("ARCHIE_PROXY_TEST_LOCAL").unwrap(),Locality::Local,None).unwrap().with_limits(Duration::from_secs(2),4096).unwrap();
    let client=InferenceClient::new(endpoint).unwrap();
    assert!(tokio::runtime::Runtime::new().unwrap().block_on(client.ready()).unwrap());
}
fn listener()->(std::net::SocketAddr,Arc<AtomicUsize>,std::thread::JoinHandle<()>) {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap(); listener.set_nonblocking(true).unwrap(); let address=listener.local_addr().unwrap();
    let hits=Arc::new(AtomicUsize::new(0)); let counter=hits.clone();
    let task=std::thread::spawn(move || {
        let started=std::time::Instant::now();
        while started.elapsed()<Duration::from_secs(4) {
            if let Ok((mut stream,_))=listener.accept() {
                stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                let mut request=Vec::new();let mut chunk=[0u8;1024];
                while request.len()<8192 && !request.windows(4).any(|w| w==b"\r\n\r\n") {
                    match stream.read(&mut chunk) {Ok(0)|Err(_)=>break,Ok(n)=>request.extend_from_slice(&chunk[..n])}
                }
                counter.fetch_add(1,Ordering::SeqCst); let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            } else {std::thread::sleep(Duration::from_millis(5));}
        }
    }); (address,hits,task)
}
#[test]
fn local_provider_transport_never_routes_through_environment_proxy() {
    let (local,local_hits,local_task)=listener(); let (proxy,proxy_hits,proxy_task)=listener();
    let output=Command::new(std::env::current_exe().unwrap()).args(["--ignored","--exact","local_proxy_worker"])
        .env("ARCHIE_PROXY_TEST_LOCAL",format!("http://{local}/v1/"))
        .env("HTTP_PROXY",format!("http://{proxy}")).env("HTTPS_PROXY",format!("http://{proxy}")).env("ALL_PROXY",format!("http://{proxy}")).env("NO_PROXY","")
        .stdout(Stdio::piped()).stderr(Stdio::piped()).output().unwrap();
    assert!(output.status.success(),"status={} stdout={} stderr={}",output.status,String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    local_task.join().unwrap(); proxy_task.join().unwrap();
    assert_eq!(proxy_hits.load(Ordering::SeqCst),0,"local context must never reach an environment proxy");assert_eq!(local_hits.load(Ordering::SeqCst),1);
}

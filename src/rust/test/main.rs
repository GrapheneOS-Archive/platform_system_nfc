//! Rootcanal HAL
//! This connects to "rootcanal" which provides a simulated
//! Nfc chip as well as a simulated environment.

use bytes::{BufMut, Bytes, BytesMut};
use nfc_packets::nci::NciChild::{InitResponse, ResetNotification, ResetResponse};
use nfc_packets::nci::ResetCommandBuilder;
use nfc_packets::nci::{NciMsgType, PacketBoundaryFlag, ResetType};
use nfc_packets::nci::{NciPacket, Packet};
use std::convert::TryInto;
use std::io::{self, ErrorKind};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, Sender};

/// Result type
type Result<T> = std::result::Result<T, CommError>;

#[derive(Debug, Error)]
enum CommError {
    #[error("Communication error")]
    IoError(#[from] io::Error),
    #[error("Packet did not parse correctly")]
    InvalidPacket,
    #[error("Packet type not supported")]
    UnsupportedPacket,
}

#[tokio::main]
async fn main() -> Result<()> {
    let (fin_tx, mut fin_rx) = mpsc::channel(1);
    let (reader, mut writer) = TcpStream::connect("127.0.0.1:54323")
        .await
        .expect("unable to create stream to rootcanal")
        .into_split();

    let mut reader = BufReader::new(reader);
    send_reset(&mut writer).await?;
    let task = tokio::spawn(async move {
        loop {
            if let Err(e) = dispatch_incoming(&mut reader, &fin_tx).await {
                match e {
                    CommError::IoError(e) if e.kind() == ErrorKind::UnexpectedEof => break,
                    _ => eprintln!("Processing error: {:?}", e),
                }
            }
        }
    });
    let msg = fin_rx.recv().await.unwrap();
    writer.write_all(msg.as_ref()).await?;
    task.await.unwrap();
    Ok(())
}

/// Send NCI events received from the HAL to the NCI layer
async fn dispatch_incoming<R>(reader: &mut R, ch: &Sender<Bytes>) -> Result<()>
where
    R: AsyncReadExt + Unpin,
{
    let mut buffer = BytesMut::with_capacity(1024);
    let t = reader.read_u8().await?;
    let len: usize = reader.read_u16().await?.into();
    eprintln!("packet {} received len={}", &t, &len);
    buffer.resize(len, 0);
    reader.read_exact(&mut buffer).await?;
    let frozen = buffer.freeze();
    eprintln!("{:?}", &frozen);
    if t == NciMsgType::Response as u8 || t == NciMsgType::Notification as u8 {
        match NciPacket::parse(&frozen) {
            Ok(p) => command_response(p, ch).await,
            Err(_) => Err(CommError::InvalidPacket),
        }
    } else {
        Err(CommError::UnsupportedPacket)
    }
}

async fn command_response(rsp: NciPacket, ch: &Sender<Bytes>) -> Result<()> {
    let id = match rsp.specialize() {
        ResetResponse(_) => "Reset",
        InitResponse(_) => "Init",
        ResetNotification(_) => {
            ch.send(get_termination_msg()).await.unwrap();
            "Reset NTF"
        }
        _ => "error",
    };
    println!("{} - response received", id);
    Ok(())
}

async fn send_reset<W>(out: &mut W) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let pbf = PacketBoundaryFlag::CompleteOrFinal;
    write_nci(
        out,
        (ResetCommandBuilder { pbf, reset_type: ResetType::ResetConfig }).build().into(),
    )
    .await?;
    Ok(())
}

fn get_termination_msg() -> Bytes {
    const TERMINATION: u8 = 4;
    let mut data = BytesMut::with_capacity(3);
    data.put_u8(TERMINATION);
    data.put_u16(0u16);
    data.freeze()
}

async fn write_nci<W>(writer: &mut W, cmd: NciPacket) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let pkt_type = cmd.get_mt() as u8;
    let b = cmd.to_bytes();
    let mut data = BytesMut::with_capacity(b.len() + 3);
    data.put_u8(pkt_type);
    data.put_u16(b.len().try_into().unwrap());
    data.extend(b);
    writer.write_all(&data[..]).await?;
    println!("Reset command is sent");
    Ok(())
}

//! This connects to "rootcanal" and provides a simulated
//! Nfc chip as well as a simulated environment.

use bytes::{BufMut, BytesMut};
use nfc_packets::nci;
use nfc_packets::nci::CommandChild::{InitCommand, ResetCommand};
use nfc_packets::nci::{
    CommandPacket, NotificationPacket, Packet, PacketBoundaryFlag, ResponsePacket,
};
use nfc_packets::nci::{
    ConfigStatus, NciVersion, ResetNotificationBuilder, ResetResponseBuilder, ResetTrigger,
    ResetType,
};
use nfc_packets::nci::{InitResponseBuilder, NfccFeatures, RfInterface};
use num_derive::{FromPrimitive, ToPrimitive};
use std::convert::TryInto;
use thiserror::Error;
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Result type
type Result<T> = std::result::Result<T, RootcanalError>;

#[derive(Debug, Error)]
enum RootcanalError {
    #[error("Termination requiest")]
    TerminateTask,
    #[error("Socket error")]
    IoError(#[from] io::Error),
    #[error("Unsupported command packet")]
    UnsupportedCommand,
    #[error("Packet did not parse correctly")]
    InvalidPacket,
    #[error("Packet type not supported")]
    UnsupportedPacket,
}

#[derive(FromPrimitive, ToPrimitive)]
enum NciPacketType {
    Data = 0x00,
    Command = 0x01,
    Response = 0x02,
    Notification = 0x03,
    Termination = 0x04,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:54323").await?;

    let (mut sock, _) = listener.accept().await?;

    tokio::spawn(async move {
        let (mut rd, mut wr) = sock.split();
        loop {
            if let Err(e) = process(&mut rd, &mut wr).await {
                match e {
                    RootcanalError::TerminateTask => break,
                    _ => panic!("Communication error: {:?}", e),
                }
            }
        }
    })
    .await?;
    Ok(())
}

async fn process<R, W>(reader: &mut R, writer: &mut W) -> Result<()>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut buffer = BytesMut::with_capacity(1024);
    let pkt_type = reader.read_u8().await?;
    let len: usize = reader.read_u16().await?.into();
    eprintln!("packet {} received len={}", &pkt_type, &len);
    buffer.resize(len, 0);
    reader.read_exact(&mut buffer).await?;
    let frozen = buffer.freeze();
    eprintln!("{:?}", &frozen);
    if pkt_type == NciPacketType::Command as u8 {
        match CommandPacket::parse(&frozen) {
            Ok(p) => command_response(writer, p).await,
            Err(_) => Err(RootcanalError::InvalidPacket),
        }
    } else if pkt_type == NciPacketType::Termination as u8 {
        Err(RootcanalError::TerminateTask)
    } else {
        Err(RootcanalError::UnsupportedPacket)
    }
}

async fn command_response<W>(out: &mut W, cmd: CommandPacket) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let pbf = PacketBoundaryFlag::CompleteOrFinal;
    match cmd.specialize() {
        ResetCommand(rst) => {
            write_rsp(out, (ResetResponseBuilder { pbf, status: nci::Status::Ok }).build()).await?;
            write_ntf(
                out,
                (ResetNotificationBuilder {
                    pbf,
                    trigger: ResetTrigger::ResetCommand,
                    config_status: if rst.get_reset_type() == ResetType::KeepConfig {
                        ConfigStatus::ConfigKept
                    } else {
                        ConfigStatus::ConfigReset
                    },
                    nci_version: NciVersion::Version20,
                    manufacturer_id: 0,
                    payload: None,
                })
                .build(),
            )
            .await
        }
        InitCommand(_) => {
            let nfcc_feat = [0u8; 5];
            let rf_int = [0u8, 2];
            write_rsp(
                out,
                (InitResponseBuilder {
                    pbf,
                    status: nci::Status::Ok,
                    nfcc_features: NfccFeatures::parse(&nfcc_feat).unwrap(),
                    max_log_conns: 0,
                    max_rout_tbls_size: 0x0000,
                    max_ctrl_payload: 255,
                    max_data_payload: 255,
                    num_of_credits: 0,
                    max_nfcv_rf_frame_sz: 64,
                    rf_interface: vec![RfInterface::parse(&rf_int).unwrap()],
                })
                .build(),
            )
            .await
        }
        _ => Err(RootcanalError::UnsupportedCommand),
    }
}

async fn write_rsp<W, T>(writer: &mut W, rsp: T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Into<ResponsePacket>,
{
    let b = rsp.into().to_bytes();
    let mut data = BytesMut::with_capacity(b.len() + 3);
    data.put_u8(NciPacketType::Response as u8);
    data.put_u16(b.len().try_into().unwrap());
    data.extend(b);
    writer.write_all(&data[..]).await?;
    println!("command written");
    Ok(())
}
async fn write_ntf<W, T>(writer: &mut W, rsp: T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Into<NotificationPacket>,
{
    let b = rsp.into().to_bytes();
    let mut data = BytesMut::with_capacity(b.len() + 3);
    data.put_u8(NciPacketType::Notification as u8);
    data.put_u16(b.len().try_into().unwrap());
    data.extend(b);
    writer.write_all(&data[..]).await?;
    println!("command written");
    Ok(())
}

use anyhow::Ok;
use config::settings;
use models::inventory::Host;
use dal::{AsEasyTransaction, new_client};
use tracing::info;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct SSHClientInfo {
    pub address: String,
    pub port: i32,
    pub user: String,
    pub password: String,
    pub writable_directory: String, // Usually just /tmp
} 


// Uses SFTP and SSH to write a given file to given server
pub async fn write_file_to_external(
    directory_path: String, // ex "/srv/www/laas_files/fedora-kickstarts/"
    file_name: String,      // ex "hpe1.ks"
    file_content: String,
    ssh_client: SSHClientInfo,

) -> Result<(), anyhow::Error> {
    info!("Attempting to connect to {} via ssh", ssh_client.address);

    let mut session =
        ssh2::Session::new().unwrap_or_else(|_| panic!("Failed to create a new SSH session for {}.", ssh_client.address));
    let connection =
        std::net::TcpStream::connect(format!("{}:{}", ssh_client.address, ssh_client.port))
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to open TCP stream to {}:{}.",
                    ssh_client.address, ssh_client.port
                )
            });

    session.set_tcp_stream(connection);
    session.handshake().unwrap();
    session
        .userauth_password(&ssh_client.user, &ssh_client.password)
        .expect("SSH basic authentication failed");

    info!("Connected to {} successfully, attempting to write via sftp", ssh_client.address);

    let sftp = session.sftp().expect("Expected to open sftp session");

    let writable_directory = ssh_client.writable_directory; // At the time of writing this is /tmp
    let temp_path = format!("{writable_directory}/{file_name}");

    // We cannot write a file with sftp with elevated (sudo) privileges (at least not easily), so we write to /tmp then copy it over with privileges
    info!("Writing given file content to {temp_path}",);
    std::io::Write::write_all(
        &mut sftp
            .open_mode(
                Path::new(&temp_path),
                ssh2::OpenFlags::CREATE | ssh2::OpenFlags::WRITE | ssh2::OpenFlags::TRUNCATE,
                0o644,
                ssh2::OpenType::File,
            )
            .unwrap(),
        file_content.as_bytes(),
    )
    .unwrap();

    info!("Was able to write a file to {} on {} successfully, copying to {}", temp_path, ssh_client.address, directory_path);

    let mut channel = session.channel_session()?;
    channel.exec(&format!("sudo cp {} {}/{}", temp_path, directory_path, file_name)).unwrap_or_else(|_| {
        panic!("Failed to write file {} to {}.",file_name, directory_path)
    });

    channel.close().unwrap();

    info!(
        "Was able to successfully write file {} to {} on {}",
        file_name, directory_path, ssh_client.address
    );

    Ok(())
}


/// Derives MAC address from host and creates a file for each in the directory for the server in the ssh_client
pub async fn write_system_grub_to_external(
    host: &Host, 
    directory_path: String, // The directory all of the files will be written to
    grub_content: String,
    ssh_client: SSHClientInfo,

) -> Result<(), anyhow::Error> {
    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    // Get mac addresses
    let host_ports = host.ports(&mut transaction).await?;
    let mac_address_filenames: Vec<String> = host_ports
        .iter()
        .map(|host_port| format!("{}", host_port.mac).to_ascii_lowercase())
        .collect();

    for filename in &mac_address_filenames {
        write_file_to_external(
            directory_path.clone(),
            filename.to_string(), 
            grub_content.clone(), 
            ssh_client.clone(),
        ).await.unwrap();
    }

    if settings().workflow_config.generate_hostname_grub_file {
        write_file_to_external(
            directory_path, 
            format!("{}.cfg",host.server_name), 
            grub_content.clone(), 
            ssh_client.clone()
        ).await.unwrap();
    }


    Ok(())
}


pub async fn cleanup_generated_host_grub_files(
    host: &Host,
    directory_path: String, // The directory all of the files will be written to (must end in trailing slash)
    ssh_client: SSHClientInfo,
) -> Result<(), anyhow::Error> {
    info!("Cleaning up host {} grub files", host.server_name);

    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    let mut session =
        ssh2::Session::new().unwrap_or_else(|_| panic!("Failed to create a new SSH session for {}.", ssh_client.address));
    let connection =
        std::net::TcpStream::connect(format!("{}:{}", ssh_client.address, ssh_client.port))
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to open TCP stream to {}:{}.",
                    ssh_client.address, ssh_client.port
                )
            });
    session.set_tcp_stream(connection);
    session.handshake().unwrap();
    session
        .userauth_password(&ssh_client.user, &ssh_client.password)
        .expect("SSH basic authentication failed");

    

    // Get mac addresses
    let host_ports = host.ports(&mut transaction).await?;
    let mac_address_filenames: Vec<String> = host_ports
        .iter()
        .map(|host_port| format!("{}", host_port.mac).to_ascii_lowercase())
        .collect();
    

    for mac_addr_filename in &mac_address_filenames {
        let mut channel = session.channel_session()?;
        let command = format!("sudo rm {}/{}", directory_path, &mac_addr_filename);
        info!("Running command '{}' on {}", command, ssh_client.address);
        channel.exec(&command).unwrap();
        channel.close()?;

    }

    Ok(())


}


pub async fn cleanup_generated_hostname_files(
    host: &Host,
    directory_paths: Vec<String>, // The directories the <hostname>.* file will be removed from (must end in trailing slash)
    ssh_client: SSHClientInfo,
) -> Result<(), anyhow::Error> {


    let mut session =
        ssh2::Session::new().unwrap_or_else(|_| panic!("Failed to create a new SSH session for {}.", ssh_client.address));
    let connection =
        std::net::TcpStream::connect(format!("{}:{}", ssh_client.address, ssh_client.port))
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to open TCP stream to {}:{}.",
                    ssh_client.address, ssh_client.port
                )
            });
    session.set_tcp_stream(connection);
    session.handshake().unwrap();
    session
        .userauth_password(&ssh_client.user, &ssh_client.password)
        .expect("SSH basic authentication failed");


    let hostname = host.server_name.clone();

    for directory in directory_paths {
        let mut channel = session.channel_session()?;
        let command = format!("sudo rm {}/{}.*", &directory, hostname);
        info!("Running command '{}' on {}", command, ssh_client.address);
        channel.exec(&command).unwrap();
        channel.close()?;
    }

    Ok(())

}
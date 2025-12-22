use http::Uri;
use config::settings;
use models::{dashboard::{Image, image::ImageKernelArg}, inventory::HostPort};
use dal::get_db_pool;
use notifications::templates::render_template;
use tera;

// Used to represent a GrubConfig file before rendering with Jinja
// Call GenericGrubConfig::<distro>() to create the Config and call self.render() to render the image
    // Arguments are used to extrapolate the required kernel args for each distro that are per-provision 
pub struct GenericGrubConfig {
    pub kernel_path: Uri, 
    pub kernel_args: Vec<String>, // Extra kernel args, required ones are handled by functions
    pub initrd_paths: Vec<Uri>,
    pub hostname: String,
}


impl GenericGrubConfig {
    pub async fn rhel(
        image: Image,
        interfaces: Vec<HostPort>,
        server_hostname: String, // ie. hpe1, arm4 etc
    ) -> Self {
        let db_pool = get_db_pool().await.unwrap();
        let mut kargs: Vec<String> = vec![];

        let mut image_kernel_args = ImageKernelArg::compile_kernel_args_for_image(&image.name, &db_pool).await.unwrap();
        kargs.append(&mut image_kernel_args);

        for port in &interfaces {
            kargs.push(format!("ifname={}:{}", port.name, port.mac.to_string()));
        }

        let pxe_settings = settings().pxe.clone();
        let kickstart_dir = pxe_settings.http_tftp_uri.rhel_kickstart;
        let pxe_address = pxe_settings.address;
        kargs.push(format!("inst.ks=http://{}{}/{}.ks", pxe_address, kickstart_dir, server_hostname));

        
        GenericGrubConfig {
            kernel_path: image.tftp_kernel_path().clone(), 
            kernel_args: kargs, 
            initrd_paths: image.tftp_initrd_paths().to_vec(), 
            hostname: server_hostname, 
        }
    }

    pub async fn ubuntu(
        image: Image,
        server_hostname: String, // ie. hpe1, arm4 etc
    ) -> Self {
        let db_pool = get_db_pool().await.unwrap();
        let mut kargs: Vec<String> = vec![];

        let mut image_kernel_args = ImageKernelArg::compile_kernel_args_for_image(&image.name, &db_pool).await.unwrap();
        kargs.append(&mut image_kernel_args);
        

        let settings_copy = settings();
        let ci_url = format!("http://{}{}/{}.yaml", settings_copy.pxe.address, settings_copy.pxe.http_tftp_uri.ubuntu_cloudinit, &server_hostname);

        kargs.push(format!("cloud-config-url={}", ci_url));
        // kargs.push(format!("provision_id={}", ID::new().to_string()));

        GenericGrubConfig {
            kernel_path: image.tftp_kernel_path().clone(), 
            kernel_args: kargs, 
            initrd_paths: image.tftp_initrd_paths().to_vec(), 
            hostname: server_hostname, 
        }

    }

    // Here to be more basic Ubuntu Grub file used pre-eve install to wipe disks
    pub async fn wipefs(
        image: Image,
        server_hostname: String, // ie. hpe1, arm4 etc
    ) -> Self {
        let db_pool = get_db_pool().await.unwrap();
        let mut kargs: Vec<String> = vec![];
        let settings_copy = settings();

        let mut image_kernel_args = ImageKernelArg::compile_kernel_args_for_image(&image.name, &db_pool).await.unwrap();
        kargs.append(&mut image_kernel_args);
        kargs.push(format!("cloud-config-url=http://{}{}", settings_copy.pxe.address, image.http_unattended_install_config_path().unwrap()));
    

        GenericGrubConfig {
            kernel_path: image.tftp_kernel_path().clone(), 
            kernel_args: kargs, 
            initrd_paths: image.tftp_initrd_paths().to_vec(), 
            hostname: server_hostname, 
        }

    }

    pub async fn eve(
        image: Image,
        server_hostname: String, // ie. hpe1, arm4 etc
        soft_serial: String,
    ) -> Self {

        let db_pool = get_db_pool().await.unwrap();
        let mut kargs: Vec<String> = vec![];

        let mut image_kernel_args = ImageKernelArg::compile_kernel_args_for_image(&image.name, &db_pool).await.unwrap();
        kargs.append(&mut image_kernel_args);

        kargs.push(format!("eve_soft_serial={soft_serial}"));

        GenericGrubConfig {
            kernel_path: image.tftp_kernel_path().clone(), 
            kernel_args: kargs, 
            initrd_paths: image.tftp_initrd_paths().to_vec(), 
            hostname: server_hostname, 
        }
    
    }

    pub fn render(&self) -> Result<String, tera::Error> {
        let mut grub_template_context = tera::Context::new();

        let mut initrd_path_str: Vec<String> = vec![];
        for path in &self.initrd_paths {
            initrd_path_str.push(path.to_string());
        }


        grub_template_context.insert("system_name", &self.hostname);
        grub_template_context.insert("kernel_path", &self.kernel_path.to_string());
        grub_template_context.insert("kernel_args", &self.kernel_args);
        grub_template_context.insert("initrd_paths", &initrd_path_str);
        

        render_template("generic/grub_config.j2", &grub_template_context)
    }
}
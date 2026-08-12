//! "Is this image pullable now?" — a manifest HEAD against the registry using
//! credentials assembled from the pod's pull secrets. Any error is reported as
//! "not available yet" so the controller retries rather than crashing.
use crate::auth::credentials_for;
use crate::imageref::ImageRef;
use oci_client::client::ClientConfig;
use oci_client::{secrets::RegistryAuth, Client, Reference};

/// Returns `true` only if the image's manifest is fetchable right now.
pub async fn is_available(image: &str, dockerconfigs: &[Vec<u8>]) -> bool {
    let parsed = ImageRef::parse(image);
    let auth = match credentials_for(&parsed.registry, dockerconfigs) {
        Some((u, p)) => RegistryAuth::Basic(u, p),
        None => RegistryAuth::Anonymous,
    };

    let reference: Reference = match image.parse() {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(image, err = %e, "unparseable image reference");
            return false;
        }
    };

    let client = Client::new(ClientConfig::default());
    match client.fetch_manifest_digest(&reference, &auth).await {
        Ok(_digest) => true,
        Err(e) => {
            tracing::debug!(image, err = %e, "image not available yet");
            false
        }
    }
}

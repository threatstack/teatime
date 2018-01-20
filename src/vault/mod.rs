use serde_json::{Value,Map};
use hyper::{Request,Response,Method};
use tokio_core::reactor::Core;
use url::Url;

use *;

header! { #[allow(missing_docs)] (XVaultToken, "X-Vault-Token") => [String] }

/// An API client for Vault
pub struct VaultClient {
    request: Option<Request>,
    api_url: Url,
    core: Core,
    token: Option<String>,
    http_client: HttpsClient,
}

impl VaultClient {
    /// Create new client
    pub fn new(api_url: &str, token: Option<String>) -> Result<Self> {
        let (client, core) = try!(<Self as ApiClient>::create_https_client());
        Ok(VaultClient {
            request: None,
            api_url: try!(Url::parse(api_url)),
            token,
            core,
            http_client: client,
        })
    }
}

impl<'a> ApiClient<'a, SerdeValue, Value> for VaultClient {
    type Params = JsonParams;

    fn request_init<T>(&mut self, method: Method, u: T) -> Result<()> where T: Into<Result<Uri>> {
        let uri = u.into()?;
        self.request = Some(Request::new(method, uri));
        Ok(())
    }

    fn get_api_url(&self) -> Url {
        self.api_url.clone()
    }

    fn get_hyper_client(&mut self) -> &mut HttpsClient {
        &mut self.http_client
    }

    fn get_core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn set_params(&mut self, params: Option<&Self::Params>) -> Result<()> {
        if let Some(ref mut request) = self.get_request() {
            if let Some(ref t) = self.token {
                let vault_token = XVaultToken(t.clone());
                request.headers_mut().set::<XVaultToken>(vault_token);
            }
            if let Some(ps) = params {
                request.set_body(ps.to_string());
            }
            Ok(())
        } else {
            Err(ClientError::new("Request not initialized"))
        }
    }

    fn get_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    fn login(&mut self, creds: &ApiCredentials) -> Result<()> {
        let username: String;
        let mut args = Map::new();
        if let ApiCredentials::UserPassTwoFactor(ref u, ref p, ref y) = *creds {
            username = u.clone();
            args.insert("password".to_string(), Value::String(p.clone()));
            args.insert("passcode".to_string(), Value::String(y.clone()));
        } else if let ApiCredentials::UserPass(ref u, ref p) = *creds {
            username = u.clone();
            args.insert("password".to_string(), Value::String(p.clone()));
        } else {
            return Err(ClientError::new("Invalid credentials provided for login"));
        }
        let token_payload = self.api_request(
            Method::Post, RequestTarget::Path(
                format!("/v1/auth/ldap/login/{}", username).as_str()
            ), Some(&JsonParams::from(args))
        )?;
        let token = try!(token_payload.get("auth").and_then(|x| x.get("client_token"))
                         .and_then(|x| x.as_str())
                         .ok_or(ClientError::new("Could not retrieve auth token")));
        self.token = Some(token.to_string());
        Ok(())
    }
}

impl JsonApiClient for VaultClient {
    fn json_api_next_page_url(&mut self, _response: &Response)
                              -> Result<Option<Url>> {
        unimplemented!()
    }
}

use serde_json::{Value,Map};
use hyper::{Request,Response,Method};

use *;

header! { #[allow(missing_docs)] (XVaultToken, "X-Vault-Token") => [String] }

/// An API client for Vault
pub struct VaultClient {
    api_uri: Uri,
    token: Option<String>,
    http_client: SimpleHttpClient,
}

impl VaultClient {
    /// Create new client
    pub fn new(api_uri: &str, token: Option<String>) -> Result<Self> {
        Ok(VaultClient {
            api_uri: api_uri.parse::<Uri>()?,
            token,
            http_client: SimpleHttpClient::new()?,
        })
    }
}

impl ApiClient<SimpleHttpClient> for VaultClient {
    type Params = JsonParams;

    fn base_uri(&self) -> &Uri {
        &self.api_uri
    }

    fn http_client(&self) -> &SimpleHttpClient {
        &self.http_client
    }

    fn http_client_mut(&mut self) -> &mut SimpleHttpClient {
        &mut self.http_client
    }

    fn set_request_attributes(request: &mut Request, params: Option<Self::Params>) -> Result<()> {
        if let Some(ps) = params {
            request.set_body(ps.to_string());
        }
        Ok(())
    }

    fn set_api_headers(&mut self) -> Result<()> {
        let token = match self.token {
            Some(ref t) => t.clone(),
            None => {
                return Err(ClientError::new("Failed to set auth token for Vault - \
                                            no auth token provided"));
            }
        };
        self.http_client_mut().set_request_header(XVaultToken(token))?;
        Ok(())
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
        let uri = format!("{}/v1/auth/ldap/login/{}", self.api_uri, username).parse::<Uri>()?;
        let token_payload = self.request_json(
            Method::Post, uri,
            Some(JsonParams::from(args))
        )?;
        let token = try!(token_payload.get("auth").and_then(|x| x.get("client_token"))
                         .and_then(|x| x.as_str())
                         .ok_or(ClientError::new("Could not retrieve auth token")));
        self.token = Some(token.to_string());
        Ok(())
    }
}

impl JsonApiClient<SimpleHttpClient> for VaultClient {
    fn next_page_uri(&mut self, _response: &Response)
                              -> Result<Option<Uri>> {
        unimplemented!()
    }
}

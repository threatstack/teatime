
use url::Url;
use hyper::{Request,Method,Uri};
use hyper::header::{ContentType,ContentLength};
use tokio_core::reactor::Core;
use serde_json::Value;

use *;

/// Sensu API client
pub struct SensuClient {
    request: Option<Request>,
    api_url: Url,
    core: Core,
    client: HttpsClient,
}

impl SensuClient {
    /// Create a new Sensu API client
    pub fn new(api_url: &str) -> Result<Self> {
        let (client, core) = <Self as ApiClient>::create_https_client()?;
        Ok(SensuClient {
            request: None,
            api_url: Url::parse(api_url)?,
            core,
            client,
        })
    }
}

impl<'a> ApiClient<'a, SerdeValue, Value> for SensuClient {
    type Params = JsonParams;

    fn get_api_url(&self) -> Url {
        self.api_url.clone()
    }

    fn get_hyper_client(&mut self) -> &mut HttpsClient {
        &mut self.client
    }

    fn get_core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn request_init<T>(&mut self, method: Method, u: T) -> Result<()> where T: Into<Result<Uri>> {
        let uri = u.into()?;
        self.request = Some(Request::new(method, uri));
        Ok(())
    }

    fn set_params(&mut self, params: Option<&Self::Params>) -> Result<()> {
        if let Some(ref mut request) = self.request {
            if let Some(ps) = params {
                let request_body = ps.to_string();
                let request_len = request_body.len();
                request.set_body(request_body);
                request.headers_mut().set(ContentType::json());
                request.headers_mut().set(ContentLength(request_len as u64));
            }
            Ok(())
        } else {
            Err(ClientError::new("Request not initialized"))
        }
    }

    fn get_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    fn login(&mut self, _creds: &ApiCredentials) -> Result<()> {
        Ok(())
    }
}

use hyper::{Request,Uri};
use hyper::header::{ContentType,ContentLength};

use *;

/// Sensu API client
pub struct SensuClient {
    api_uri: Uri,
    client: SimpleHttpClient,
}

impl SensuClient {
    /// Create a new Sensu API client
    pub fn new(api_uri: &str) -> Result<Self> {
        Ok(SensuClient {
            api_uri: api_uri.parse::<Uri>()?,
            client: SimpleHttpClient::new()?,
        })
    }
}

impl ApiClient<SimpleHttpClient> for SensuClient {
    type Params = JsonParams;

    fn base_uri(&self) -> &Uri {
        &self.api_uri
    }

    fn http_client(&self) -> &SimpleHttpClient {
        &self.client
    }

    fn http_client_mut(&mut self) -> &mut SimpleHttpClient {
        &mut self.client
    }

    fn set_request_attributes(request: &mut Request, params: Option<Self::Params>) -> Result<()> {
        if let Some(ps) = params {
            let request_body = ps.to_string();
            request.headers_mut().set(ContentLength(request_body.len() as u64));
            request.set_body(request_body);
        }
        Ok(())
    }

    fn set_api_headers(&mut self) -> Result<()> {
        self.http_client_mut().set_request_header(ContentType::json())?;
        Ok(())
    }

    fn login(&mut self, _creds: &ApiCredentials) -> Result<()> {
        Ok(())
    }
}

impl JsonApiClient<SimpleHttpClient> for SensuClient {
    fn next_page_uri(&mut self, _resp: &Response) -> Result<Option<Uri>> {
        Ok(None)
    }
}

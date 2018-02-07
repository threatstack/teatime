use hyper::{Uri};
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
    fn base_uri(&self) -> &Uri {
        &self.api_uri
    }

    fn http_client(&self) -> &SimpleHttpClient {
        &self.client
    }

    fn http_client_mut(&mut self) -> &mut SimpleHttpClient {
        &mut self.client
    }

    fn request_future<B>(&mut self, method: Method, uri: Uri, body: Option<B>) -> Option<FutureResponse>
            where B: ToString {
        let body_len = match body {
            Some(ref b) => b.to_string().len(),
            None => 0,
        };

        let full_uri = self.full_uri(uri).ok()?;
        let client = self.http_client_mut();
        client.start_request(method, full_uri)
            .add_header(ContentLength(body_len as u64)).add_header(ContentType::json());
        if let Some(ref b) = body {
            client.add_body(b.to_string());
        }
        client.make_request().future()
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

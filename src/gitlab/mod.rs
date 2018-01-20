use std::str;
use std::fmt;
use std::collections::HashMap;

use serde_json::{Value,Map};
use hyper::{self,Client,Request,Method,Response};
use hyper::client::HttpConnector;
use hyper::header::{self,Header,Raw,ContentType,Authorization,Bearer};
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Core;
use url::Url;

use *;

named!(parse_link_header<&str, HashMap<String, String> >,
    fold_many1!(
        ws!(do_parse!(
            opt!(tag!(",")) >>
            tag!("<") >>
            link: take_until!(">;") >>
            tag!(">;") >>
            tag!(r#"rel=""#) >>
            position: take_until!(r#"""#) >>
            tag!(r#"""#) >>
            (position, link)
        )),
        HashMap::new(),
        |mut hm: HashMap<String, String>, (position, link): (&str, &str)| {
            hm.insert(position.to_string(), link.to_string());
            hm
        }
    )
);

/// Struct representing the pagination header in Gitlab
#[derive(Clone)]
pub struct Link {
    previous: Option<String>,
    next: Option<String>,
    first: Option<String>,
    last: Option<String>,
}

/// Trait for handling Gitlab's pagination format
pub trait HasNextLink {
    /// True if there is another page available
    fn has_next(&self) -> bool;
    /// Get URL of next page
    fn next(&self) -> Option<Url>;
}

impl<'a> HasNextLink for Option<&'a Link> {
    fn has_next(&self) -> bool {
        match *self {
            Some(ref l) => l.next.is_some(),
            _ => false
        }
    }

    fn next(&self) -> Option<Url> {
        match *self {
            Some(ref l) => match l.next {
                Some(ref n) => Url::parse(n.as_str()).ok(),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Header for Link {
    fn header_name() -> &'static str {
        "Link"
    }

    fn parse_header(raw: &Raw) -> hyper::Result<Self> {
        let bytes = match raw.one() {
            Some(b) => b,
            _ => { return Err(hyper::error::Error::Header); },
        };
        let string = match str::from_utf8(bytes) {
            Ok(s) => s,
            _ => { return Err(hyper::error::Error::Header); }
        };
        let mut hm = match parse_link_header(string).to_result() {
            Ok(hash) => hash,
            _ => { return Err(hyper::error::Error::Header); },
        };
        Ok(Link {
            previous: hm.remove("prev"),
            next: hm.remove("next"),
            first: hm.remove("first"),
            last: hm.remove("last"),
        })
    }

    fn fmt_header(&self, f: &mut header::Formatter) -> fmt::Result {
        let _ = f;
        unimplemented!()
    }
}

/// Gitlab API client
pub struct GitlabClient {
    request: Option<Request>,
    host_url: Url,
    token: Option<String>,
    core: Core,
    client: Client<HttpsConnector<HttpConnector>>,
}

impl<'a> GitlabClient {
    /// Create a new Gitlab API client
    pub fn new(host_url: String) -> Result<Self> {
        let (client, core) = try!(<Self as ApiClient>::create_https_client());
        Ok(GitlabClient{
            request: None,
            token: None,
            host_url: try!(Url::parse(host_url.as_str())),
            core,
            client,
        })
    }
}

impl<'a> ApiClient<'a, SerdeValue, Value> for GitlabClient {
    type Params = JsonParams;

    fn request_init<T>(&mut self, method: Method, u: T) -> Result<()> where T: Into<Result<Uri>> {
        let uri = u.into()?;
        self.request = Some(Request::new(method, uri));
        Ok(())
    }

    fn get_api_url(&self) -> Url {
        self.host_url.clone()
    }

    fn get_hyper_client(&mut self) -> &mut Client<HttpsConnector<HttpConnector>> {
        &mut self.client
    }

    fn get_core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn set_params(&mut self, params: Option<&Self::Params>) -> Result<()> {
        if let Some(ref mut req) = self.request {
            if let Some(ps) = params {
                req.set_body(ps.to_string())
            }
            req.headers_mut().set(ContentType::json());
            if let Some(ref t) = self.token {
                req.headers_mut().set(Authorization(Bearer{ token: t.clone() }));
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
        let token = {
            let mut auth = |user: &String, pass: &String| -> Result<Option<String>> {
                let mut json_map = Map::new();
                json_map.insert("grant_type".to_string(), Value::from("password"));
                json_map.insert("username".to_string(), Value::from(user.clone()));
                json_map.insert("password".to_string(), Value::from(pass.clone()));
                let origin = self.get_api_url().origin().ascii_serialization();
                let target = RequestTarget::from(origin.as_str()) + "/oauth/token";
                let json = try!(self.api_request(Method::Post, target?,
                                                 Some(&JsonParams::from(json_map))));
                let token_json = try!(json.get("access_token")
                                 .ok_or(ClientError::new("Could not log in with given username and password")))
                                 .as_str().map(|x| { x.to_string() });
                Ok(token_json)
            };

            match *creds {
                ApiCredentials::NoAuth => None,
                ApiCredentials::UserPass(ref user, ref pass) => {
                    try!(auth(user, pass))
                },
                ApiCredentials::UserPassTwoFactor(ref user, ref pass, _) => {
                    try!(auth(user, pass))
                },
                ApiCredentials::ApiKey(ref key) => Some(key.clone()),
            }
        };

        self.token = token;
        Ok(())
    }
}

impl JsonApiClient for GitlabClient {
    fn json_api_next_page_url(&mut self, resp: &Response)
                              -> Result<Option<Url>> {
        let link_option = resp.headers().get::<Link>();
        Ok(link_option.next())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parsing_macro() {
        let hm = parse_link_header(r#"<https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=1&per_page=3>; rel="prev", <https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=3&per_page=3>; rel="next", <https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=1&per_page=3>; rel="first", <https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=3&per_page=3>; rel="last""#).to_result().unwrap();
        assert_eq!(*hm.get(&"prev".to_string()).unwrap(), "https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=1&per_page=3".to_string());
        assert_eq!(*hm.get(&"next".to_string()).unwrap(), "https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=3&per_page=3".to_string());
        assert_eq!(*hm.get(&"first".to_string()).unwrap(), "https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=1&per_page=3".to_string());
        assert_eq!(*hm.get(&"last".to_string()).unwrap(), "https://gitlab.example.com/api/v4/projects/8/issues/8/notes?page=3&per_page=3".to_string());
    }
}

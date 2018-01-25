use std::str;
use std::fmt;
use std::collections::HashMap;

use serde_json::{Value,Map};
use hyper::{self,Request,Response};
use hyper::header::{self,Header,Raw,ContentType,Authorization,Bearer};

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
    fn next(&self) -> Option<Uri>;
}

impl<'a> HasNextLink for Option<&'a Link> {
    fn has_next(&self) -> bool {
        match *self {
            Some(ref l) => l.next.is_some(),
            _ => false
        }
    }

    fn next(&self) -> Option<Uri> {
        match *self {
            Some(ref l) => match l.next {
                Some(ref n) => n.parse::<Uri>().ok(),
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

    fn fmt_header(&self, _f: &mut header::Formatter) -> fmt::Result {
        Ok(())
    }
}

/// Gitlab API client
pub struct GitlabClient {
    base_uri: Uri,
    token: Option<String>,
    client: SimpleHttpClient,
}

impl<'a> GitlabClient {
    /// Create a new Gitlab API client
    pub fn new(base_uri: String) -> Result<Self> {
        Ok(GitlabClient{
            token: None,
            base_uri: base_uri.parse::<Uri>()?,
            client: SimpleHttpClient::new()?,
        })
    }
}

impl ApiClient<SimpleHttpClient> for GitlabClient {
    type Params = JsonParams;

    fn base_uri(&self) -> &Uri {
        &self.base_uri
    }

    fn http_client(&self) -> &SimpleHttpClient {
        &self.client
    }

    fn http_client_mut(&mut self) -> &mut SimpleHttpClient {
        &mut self.client
    }

    fn set_request_attributes(request: &mut Request, params: Option<Self::Params>) -> Result<()> {
        if let Some(ps) = params {
            request.set_body(ps.to_string())
        }
        request.headers_mut().set(ContentType::json());
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
        self.http_client_mut().set_request_header(Authorization(Bearer{ token }))?;
        Ok(())
    }

    fn login(&mut self, creds: &ApiCredentials) -> Result<()> {
        let token = {
            let mut auth = |user: &String, pass: &String| -> Result<Option<String>> {
                let mut json_map = Map::new();
                json_map.insert("grant_type".to_string(), Value::from("password"));
                json_map.insert("username".to_string(), Value::from(user.clone()));
                json_map.insert("password".to_string(), Value::from(pass.clone()));
                let uri = (self.base_uri.to_string() + "/oauth/token").parse::<Uri>()?;
                let json = <Self as JsonApiClient<SimpleHttpClient>>::request_json(self, Method::Post, uri,
                    Some(JsonParams::from(json_map)))?;
                let token_json = json.get("access_token")
                                 .ok_or(ClientError::new("Could not log in with given username and password"))?
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

impl JsonApiClient<SimpleHttpClient> for GitlabClient {
    fn next_page_uri(&mut self, resp: &Response)
                     -> Result<Option<Uri>> {
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

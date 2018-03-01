use std::str;
use std::fmt;
use std::collections::HashMap;

use serde_json::{Value,Map};
use hyper::{self,Response};
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

header! { #[allow(missing_docs)] (PrivateToken, "Private-Token") => [String] }

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

/// Support OAuth tokens and personal access tokens in Gitlab
#[derive(Clone)]
pub enum TokenType {
    /// OAuth token
    Oauth(String),
    /// Personal access tokens in Gitlab
    PersonalAccess(String),
}

/// Gitlab API client
pub struct GitlabClient {
    base_uri: Uri,
    token: Option<TokenType>,
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
    fn base_uri(&self) -> &Uri {
        &self.base_uri
    }

    fn http_client(&self) -> &SimpleHttpClient {
        &self.client
    }

    fn http_client_mut(&mut self) -> &mut SimpleHttpClient {
        &mut self.client
    }

    fn login(&mut self, creds: &ApiCredentials) -> Result<()> {
        let token = {
            let mut auth = |user: &String, pass: &String| -> Result<Option<String>> {
                let mut json_map = Map::new();
                json_map.insert("grant_type".to_string(), Value::from("password"));
                json_map.insert("username".to_string(), Value::from(user.clone()));
                json_map.insert("password".to_string(), Value::from(pass.clone()));
                let mut host_uri = format!("{}://{}", self.base_uri.scheme().ok_or(ClientError::new("Invalid base URI"))?,
                                           self.base_uri.authority().ok_or(ClientError::new("Invalid base URI"))?);
                if host_uri.ends_with('/') {
                    let _ = host_uri.pop();
                }
                let uri = (host_uri + "/oauth/token").parse::<Uri>()?;
                let json = <Self as JsonApiClient<SimpleHttpClient>>::request_json(self, Method::Post, uri,
                    Some(Value::from(json_map)))?;
                let token_json = json.get("access_token")
                                 .ok_or(ClientError::new("Could not log in with given username and password"))?
                                 .as_str().map(|x| { x.to_string() });
                Ok(token_json)
            };

            match *creds {
                ApiCredentials::NoAuth => None,
                ApiCredentials::UserPass(ref user, ref pass) => {
                    try!(auth(user, pass)).map(TokenType::Oauth)
                },
                ApiCredentials::UserPassTwoFactor(ref user, ref pass, _) => {
                    try!(auth(user, pass)).map(TokenType::Oauth)
                },
                ApiCredentials::ApiKey(ref key) => Some(TokenType::PersonalAccess(key.clone())),
            }
        };

        self.token = token;
        Ok(())
    }

    fn request_future<B>(&mut self, method: Method, uri: Uri, body: Option<B>) -> Option<FutureResponse>
            where B: ToString {
        let token = self.token.clone();
        let full_uri = self.full_uri(uri).ok()?;
        let client = self.http_client_mut();
        client.start_request(method, full_uri).add_header(ContentType::json());
        if let Some(TokenType::Oauth(ref t)) = token {
            client.add_header(Authorization(Bearer { token: t.clone() }));
        } else if let Some(TokenType::PersonalAccess(ref t)) = token {
            client.add_header(PrivateToken(t.clone()));
        }
        if let Some(b) = body {
            client.add_body(b.to_string());
        }
        client.make_request().future()
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

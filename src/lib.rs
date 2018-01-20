//! # teatime
//! An abstraction library for simplifying REST API implementations
//!
//! ## Motivation
//! When writing tools in Rust that talk with REST API endpoints,
//! there is often a lot of boilerplate code needed when using `hyper`
//! for HTTP requests. Much of this has to do with writing code
//! to handle futures, something that is not always transparent to
//! programmers coming from an imperative language background. This
//! library abstracts away the future handling and allows the user
//! to define a struct, implement this trait for the struct,
//! and then define methods for accessing the struct fields,
//! setting parameters for the HTTP request, defining headers
//! and other common REST API flows. All of the future handling is already
//! implemented as well as some additional helpful flows for things like
//! JSON API pagination.
//!
//! ## Reference implementations
//! There are three reference implementations included, one for Sensu,
//! one for Gitlab, and one for Vault. This is probably the best example
//! of common patterns for defining to required methods that do not have
//! default implementations.
//!
//! ## Using teatime
//! The bulk of teatime is driven through the `ApiClient` and `JsonApiClient`
//! traits. There are additional data structures to help with type safety
//! when dealing with REST APIs that have a very loose type model.
//!
//! See the documentation for `ApiClient` and `JsonApiClient` as well as all of
//! data structures defined in `lib.rs` as these will outline parameter types,
//! return types and required implementation bits.

#![deny(missing_docs)]

extern crate futures;
#[allow(unused_imports)]
#[macro_use]
extern crate hyper;
extern crate hyper_tls;
extern crate native_tls;
extern crate tokio_core;
#[macro_use]
extern crate serde_json;

#[cfg(feature = "gitlab")]
#[macro_use]
extern crate nom;

extern crate rpassword;
extern crate url;

/// Gitlab API client
#[cfg(feature = "gitlab")]
pub mod gitlab;
/// Sensu API client
#[cfg(feature = "sensu")]
pub mod sensu;
/// Vault API client
#[cfg(feature = "vault")]
pub mod vault;
/// Methods for entering credentials interactively
pub mod interactive;

use std::error::Error;
use std::io;
use std::num;
use std::ops::Add;
use std::fmt::{self,Formatter,Display};

use serde_json::{Value,Map};
use hyper::{Client,Method,Request,Response,Chunk,Uri};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Core;
use futures::Stream;
use url::Url;

use interactive::interactive_text;

#[macro_export]
macro_rules! error_impl {
    ($error:ident, $( $from_error:path ),* ) => {
        /// Custom error type
        #[derive(Debug,PartialEq,Eq)]
        pub struct $error(String);

        impl $error {
            /// Create new error from a type able to be converted to a `String`
            pub fn new<S>(inner_err: S) -> Self where S: Into<String> {
                $error(inner_err.into())
            }
        }

        $(
            impl From<$from_error> for $error {
                fn from(e: $from_error) -> Self {
                    $error::new(e.description())
                }
            }
        )*

        impl Error for $error {
            fn description(&self) -> &str {
                self.0.as_str()
            }
        }

        impl Display for $error {
            fn fmt(&self, f: &mut Formatter) -> fmt::Result {
                write!(f, "{}", <Self as Error>::description(self))
            }
        }
    }
}

#[macro_export]
macro_rules! pairs_to_params {
    ( $hm:ident; $( $keys:tt => $vals:expr ),* ) => {
        $(
            $hm.insert($keys.to_string(), Value::from($vals.clone()));
        )*
    };
}

error_impl!(ClientError, serde_json::Error, hyper::Error, hyper::error::UriError,
            native_tls::Error, num::ParseIntError, url::ParseError);

/// Result with `Error` type defined
pub type Result<T> = std::result::Result<T, ClientError>;

/// An enum representing three types of credentials or no authentication
#[derive(Debug,PartialEq,Eq)]
pub enum ApiCredentials {
    /// No authentication
    NoAuth,
    /// API key
    ApiKey(String),
    /// Username and password
    UserPass(String, String),
    /// Username, password, and two factor authentication
    UserPassTwoFactor(String, String, String),
}

impl ApiCredentials {
    /// Interactively prompt for username and password
    pub fn interactive_get_uname_pw() -> std::result::Result<(String, String), io::Error> {
        let username = interactive_text("Username: ")?;
        let pass = rpassword::prompt_password_stdout("Password: ")?;
        Ok((username, pass))
    }

    /// Interactively prompt for two two factor authentication
    pub fn interactive_get_2fa() -> std::result::Result<String, io::Error> {
        interactive_text("2FA: ")
    }

    /// Interactively prompt for username and password with optional two factor prompt
    pub fn interactive_get(need_2fa: bool) -> std::result::Result<Self, io::Error> {
        println!("Please enter credentials to proceed");
        let (username, pass) = ApiCredentials::interactive_get_uname_pw()?;

        if need_2fa {
            let twofactor = ApiCredentials::interactive_get_2fa()?;
            Ok(ApiCredentials::UserPassTwoFactor(username, pass, twofactor))
        } else {
            Ok(ApiCredentials::UserPass(username, pass))
        }
    }
}

/// Type alias for HTTPS client
pub type HttpsClient = Client<HttpsConnector<HttpConnector>>;

/// A struct to simplify JSON parameter handling for APIs that accept parameters as JSON objects
#[derive(Debug)]
pub struct JsonParams(Map<String, Value>);

impl Display for JsonParams {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", Value::from(self.0.clone()).to_string())
    }
}

impl From<Map<String, Value>> for JsonParams {
    fn from(v: Map<String, Value>) -> Self {
        JsonParams(v)
    }
}

/// An intermediate newtype struct to facilitate conversion from `hyper`'s `Chunk` type to
/// `serde_json`'s `Value` type
pub struct SerdeValue(Value);

impl Display for SerdeValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.to_string())
    }
}

impl From<Chunk> for SerdeValue {
    fn from(v: Chunk) -> Self {
        let json = serde_json::from_slice(&v).unwrap_or(json!({}));
        SerdeValue(json)
    }
}

impl Into<Value> for SerdeValue {
    fn into(self) -> Value {
        self.0
    }
}

/// Target URL or endpoint of the HTTP request
pub enum RequestTarget<'a> {
    /// A URL path segment with a leading `/`
    Path(&'a str),
    /// An absolute URL
    Absolute(Url),
}

impl<'a> Add<&'a str> for RequestTarget<'a> {
    type Output = Result<Self>;

    fn add(self, rhs: &'a str) -> Self::Output {
        self + RequestTarget::Path(rhs)
    }
}

impl<'a> Add for RequestTarget<'a> {
    type Output = Result<Self>;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (RequestTarget::Absolute(a), RequestTarget::Path(p)) => {
                if let Some(s) = p.get(1..) {
                    Ok(a.join(&s).map(RequestTarget::Absolute)?)
                } else {
                    Err(ClientError::new("Path appended to URL is empty"))
                }
            },
            _ => Err(ClientError::new("Addition operator must be used with Absolute URL on left and Path on the right")),
        }
    }
}

impl<'a> RequestTarget<'a> {
    /// Return `&str` representation of a `RequestTarget`
    pub fn as_str(&self) -> &str {
        match *self {
            RequestTarget::Path(ref p) => p,
            RequestTarget::Absolute(ref u) => u.as_str(),
        }
    }
}

impl<'a> Display for RequestTarget<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<'a> From<&'a str> for RequestTarget<'a> {
    fn from(v: &'a str) -> Self {
        Url::parse(v).ok().map_or(RequestTarget::Path(v), RequestTarget::Absolute)
    }
}

impl<'a> Into<Result<Uri>> for RequestTarget<'a> {
    fn into(self) -> Result<Uri> {
        match self {
            RequestTarget::Path(_) => Err(ClientError::new("Relative URL provided")),
            RequestTarget::Absolute(u) => Ok(u.to_string().parse::<Uri>()?),
        }
    }
}

/// Provides default implementations for handling future logic in `hyper` request and response flows
pub trait ApiClient<'a, I = SerdeValue, R = Value> where I: From<Chunk>, I: Into<R> {
    /// API parameter type that can be anything that can be represented as a `String`
    type Params: ToString;

    /// Accessor for the full URL of the API base endpoint
    fn get_api_url(&self) -> Url;
    /// Accessor for the hyper HTTPS client
    fn get_hyper_client(&mut self) -> &mut HttpsClient;
    /// Accessor for the underlying `tokio` event loop object
    fn get_core_mut(&mut self) -> &mut Core;
    /// Do any initial work required to set up the hyper `Request` object - the `T` type
    /// parameter can be any type that can be converted to a `Result<Uri>` value
    fn request_init<T>(&mut self, Method, T) -> Result<()> where T: Into<Result<Uri>>;
    /// Set the API parameters in the `Request` object
    fn set_params(&mut self, Option<&Self::Params>) -> Result<()>;
    /// Accessor for the Request object stored in the struct
    fn get_request(&mut self) -> Option<Request>;
    /// Define the login flow for the API - may simply return `Ok(())` for unauthenticated APIs
    fn login(&mut self, &ApiCredentials) -> Result<()>;

    /// Get the API URL as a `RequestTarget` object
    fn get_api_url_as_target(&self) -> RequestTarget<'a> {
        RequestTarget::Absolute(self.get_api_url())
    }

    /// Lower level API request that returns a hyper `Response` object - useful for parsing responses
    /// at the HTTP layer as opposed to the API flow level
    fn hyper_api_request<T>(&mut self, method: Method, target: T,
                            params: Option<&Self::Params>)
                            -> Result<Response> where T: Into<RequestTarget<'a>> {
        let mut rtarget = target.into();
        rtarget = {
            let t = match rtarget {
                RequestTarget::Path(s) => self.get_api_url_as_target() + s,
                a => Ok(a),
            };
            t?
        };
        self.request_init(method, rtarget)?;
        self.set_params(params)?;
        if let Some(request) = self.get_request() {
            let response_fut = self.get_hyper_client().request(request);
            Ok(try!(self.get_core_mut().run(response_fut)))
        } else {
            Err(ClientError::new("Request not initialized"))
        }
    }

    /// Top level API request method that takes an HTTP method, any struct that can be converted to
    /// a `RequestTarget` object, and the defined parameter type with the defined return type
    fn api_request<T>(&mut self, method: Method, target: T,
                      params: Option<&Self::Params>) -> Result<R>
                      where T: Into<RequestTarget<'a>> {
        let response = try!(self.hyper_api_request(method, target, params));
        let chunk = try!(self.api_response_to_chunk(response));
        let i_val = From::from(chunk);
        let r_val = I::into(i_val);
        Ok(r_val)
    }

    /// Default implementation for reassembling HTTP response chunks into a single
    /// object
    fn api_response_to_chunk(&mut self, resp: Response) -> Result<Chunk> {
        Ok(try!(self.get_core_mut().run(resp.body().concat2())))
    }

    /// Handle implementation details of creating an HTTPS client and return the client as well
    /// as the underlying Tokio `Core` object required for driving the client
    fn create_https_client() -> Result<(HttpsClient, Core)> {
        let core = match Core::new() {
            Ok(core) => core,
            Err(e) => {
                return Err(ClientError::new(
                        format!("Failed to start Tokio event loop: {}", e.description())
                ));
            },
        };
        let https_conn = try!(HttpsConnector::new(4, &core.handle()));
        let client = Client::configure().connector(https_conn).build(&core.handle());
        Ok((client, core))
    }
}

/// Provides a default implementation for pagination in JSON API flows
pub trait JsonApiClient: for<'a> ApiClient<'a, SerdeValue, Value> {
    /// Retrieves a URL for the request to get the next page in
    /// a paginated response
    fn json_api_next_page_url(&mut self, resp: &Response)
                              -> Result<Option<Url>>;

    /// Default implementation for handling pagination in JSON API contexts that will retrieve and
    /// parse all pages - *should be overriden if page-by-page behavior is required*
    fn json_api_paginated<'a, T>(&mut self, method: Method, target: T,
                                 params: Option<&<Self as ApiClient<'a, SerdeValue, Value>>::Params>)
                                 -> Result<Value> where T: Into<RequestTarget<'a>> {
        let mut vec: Vec<Value> = Vec::new();
        let mut response = <Self as ApiClient<'a>>::hyper_api_request(self, method.clone(), target, params)?;
        while let Some(page) = try!(self.json_api_next_page_url(&response)) {
            let chunk = try!(self.api_response_to_chunk(response));
            let json: SerdeValue = From::from(chunk);
            vec.push(json.into());
            response = try!(<Self as ApiClient<SerdeValue, Value>>::
                            hyper_api_request(self, method.clone(),
                                              RequestTarget::Absolute(page),
                                              params));
        }
        let chunk = try!(self.api_response_to_chunk(response));
        let json: SerdeValue = From::from(chunk);
        vec.push(json.into());
        Ok(Value::Array(vec))
    }

    /// Performs a conversion from a raw HTTP chunk to a parsed
    /// JSON object
    fn response_to_json(resp: Chunk) -> Result<Value> {
        let json = match resp.is_empty() {
            false => serde_json::from_slice(&resp),
            true => Ok(json!([]))
        };
        Ok(try!(json))
    }
}

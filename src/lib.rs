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
extern crate serde_json;

#[cfg(feature = "gitlab")]
#[macro_use]
extern crate nom;

extern crate rpassword;

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
use std::fmt::{self,Formatter,Display};
use std::result;
use std::str;

use serde_json::{Value,Map};
use hyper::{Client,Method,Request,Response,Chunk,Uri};
use hyper::client::{HttpConnector,FutureResponse};
use hyper::header::Header;
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Core;
use futures::{Future,Stream};

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
            native_tls::Error, num::ParseIntError);

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
#[derive(Debug,Clone)]
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

/// Methods defining low-level HTTP handling
pub trait HttpClient {
    /// Handle implementation details of creating an HTTPS client and return the client as well
    /// as the underlying Tokio `Core` object required for driving the client
    fn create_https_client(threads: usize) -> Result<(HttpsClient, Core)> {
        let core = match Core::new() {
            Ok(core) => core,
            Err(e) => {
                return Err(ClientError::new(
                        format!("Failed to start Tokio event loop: {}", e.description())
                ));
            },
        };
        let https_conn = HttpsConnector::new(threads, &core.handle())?;
        let client = Client::configure().connector(https_conn).build(&core.handle());
        Ok((client, core))
    }

    /// Initialize request
    fn request_init(&mut self, Method, Uri) -> Result<()>;
    /// Add request parameters and headers
    fn request_attributes<T>(&mut self, Option<T::Params>) -> Result<()>
        where T: ?Sized + ApiClient<Self>;
    /// Set an individual header in the HTTP request
    fn set_request_header<H>(&mut self, H) -> Result<()>
        where H: Header;
    /// Make HTTP request
    fn make_request(&mut self) -> Result<()>;
    /// Evaluate a `hyper` future
    fn evaluate_future<F>(&mut self, future: F) -> result::Result<F::Item, F::Error> where F: Future;
    /// Get complete HTTP response
    fn response(&mut self) -> Result<Response>;

    /// Convert `Response` object to a reassembled `Chunk` type
    fn response_to_body(&mut self, resp: Response) -> Result<Chunk> {
        self.evaluate_future(resp.body().concat2()).map_err(|e| ClientError::new(format!("{}", e)))
    }

    /// Get request response body only
    fn body(&mut self) -> Result<Chunk> {
        let resp = self.response()?;
        self.response_to_body(resp)
    }
}

/// Reference implementation of `HttpClient` trait - should be good enough for most use cases
pub struct SimpleHttpClient {
    https_client: HttpsClient,
    core: Core,
    request: Option<Request>,
    response_fut: Option<FutureResponse>,
}

impl SimpleHttpClient {
    /// Create a new `SimpleHttpClient`
    pub fn new() -> Result<Self> {
        let (https_client, core) = <Self as HttpClient>::create_https_client(4)?;
        Ok(SimpleHttpClient { https_client, core, request: None, response_fut: None })
    }
}

impl HttpClient for SimpleHttpClient {
    fn request_init(&mut self, method: Method, uri: Uri) -> Result<()> {
        self.request = Some(Request::new(method, uri));
        Ok(())
    }

    fn request_attributes<T>(&mut self, params: Option<T::Params>) -> Result<()>
            where T: ?Sized + ApiClient<Self> {
        if let Some(ref mut req) = self.request {
            if params.is_some() {
                T::set_request_attributes(req, params)?;
            }
            Ok(())
        } else {
            Err(ClientError::new("Request not initialized, cannot set parameters"))
        }
    }

    fn set_request_header<H>(&mut self, header: H) -> Result<()> where H: Header {
        if let Some(ref mut req) = self.request {
            req.headers_mut().set(header);
            Ok(())
        } else {
            Err(ClientError::new("Request not initialized, cannot set header"))
        }
    }

    fn make_request(&mut self) -> Result<()> {
        if let Some(req) = self.request.take() {
            self.response_fut = Some(self.https_client.request(req));
            Ok(())
        } else {
            Err(ClientError::new("Request not initialized, cannot make request"))
        }
    }

    fn evaluate_future<F>(&mut self, future: F) -> result::Result<F::Item, F::Error> where F: Future {
        self.core.run(future)
    }

    fn response(&mut self) -> Result<Response> {
        match self.response_fut.take() {
            Some(fut) => Ok(self.evaluate_future(fut)?),
            None => Err(ClientError::new("No request sent, get response")),
        }
    }
}

/// Provides default implementations for handling future logic in `hyper` request and response flows
pub trait ApiClient<HTTP> where HTTP: ?Sized + HttpClient {
    /// API parameter type that can be anything that can be represented as a `String`
    type Params: ToString + Clone;

    /// Get base API URI to which all relative endpoint requests will be appended
    fn base_uri(&self) -> &Uri;
    /// Get underlying HTTP client
    fn http_client(&self) -> &HTTP;
    /// Get underlying HTTP client mutably
    fn http_client_mut(&mut self) -> &mut HTTP;
    /// Set the `Request` object attributes directly such as the request body
    /// - defined at the API level so that `SimpleHttpClient` can work with any API
    fn set_request_attributes(&mut Request, Option<Self::Params>) -> Result<()>;
    /// Set the API headers most likely using `SimpleHttpClient`'s `set_request_header()` API -
    /// mainly intended for headers like auth-related tokens
    fn set_api_headers(&mut self) -> Result<()>;
    /// Define the login flow for the API - may simply return `Ok(())` for unauthenticated APIs
    fn login(&mut self, &ApiCredentials) -> Result<()>;

    /// Make API request and get `Response`
    fn request(&mut self, method: Method, uri: Uri, params: Option<Self::Params>) -> Result<Response> {
        let is_absolute_uri = uri.is_absolute();
        let full_uri = if !is_absolute_uri {
            let mut no_leading_slash_uri = uri.as_ref();
            if let "/" = &no_leading_slash_uri[..1] {
                no_leading_slash_uri = &no_leading_slash_uri[1..];
            }
            let mut base_uri = self.base_uri().to_string();
            if !base_uri.ends_with("/") {
                base_uri.push('/');
            }
            let uri_concat = base_uri + no_leading_slash_uri;
            uri_concat.parse::<Uri>()?
        } else {
            uri
        };
        // Used in an inner block due to mutability requirements
        {
            let http_client = self.http_client_mut();
            http_client.request_init(method, full_uri)?;
            http_client.request_attributes::<Self>(params)?;
        }
        self.set_api_headers()?;
        let http_client = self.http_client_mut();
        http_client.make_request()?;
        http_client.response()
    }

    /// Make API request and get `Chunk`
    fn request_to_response_body(&mut self, method: Method, uri: Uri, params: Option<Self::Params>) -> Result<Chunk> {
        let response = self.request(method, uri, params)?;
        self.http_client_mut().response_to_body(response)
    }
}

/// Provides a default implementation for pagination in JSON API flows
pub trait JsonApiClient<HTTP>: ApiClient<HTTP> where HTTP: HttpClient {
    /// Retrieves a URL for the request to get the next page in
    /// a paginated response
    fn next_page_uri<'a>(&mut self, resp: &Response)
                         -> Result<Option<Uri>>;

    /// Default implementation to make an API request and convert the response to JSON
    fn request_json(&mut self, method: Method, uri: Uri, params: Option<Self::Params>) -> Result<Value> {
        let response = self.request(method, uri, params)?;
        self.response_to_json(response)
    }

    /// Default implementation for handling pagination in JSON API contexts that will retrieve and
    /// parse all pages - *should be overriden if page-by-page behavior is required*
    fn autopagination<T>(&mut self, method: Method, target: Uri,
                         params: Option<<Self as ApiClient<HTTP>>::Params>)
                         -> Result<Value> {
        let mut vec: Vec<Value> = Vec::new();
        let mut response = <Self as ApiClient<HTTP>>::request(
            self, method.clone(), target.clone(), params.clone()
        )?;
        while let Some(page) = try!(self.next_page_uri(&response)) {
            let json = self.response_to_json(response)?;
            vec.push(json);
            response = <Self as ApiClient<HTTP>>::request(self, method.clone(), page, params.clone())?;
        }
        let chunk = self.http_client_mut().response_to_body(response)?;
        let json = serde_json::from_slice(&chunk)?;
        vec.push(json);
        Ok(Value::Array(vec))
    }

    /// Convert a response body directly to JSON
    fn response_to_json(&mut self, response: Response) -> Result<Value> {
        let chunk = self.http_client_mut().response_to_body(response)?;
        serde_json::from_slice(&chunk).map_err(|_e| {
            let string_body = match str::from_utf8(&chunk) {
                Ok(s) => s,
                _ => { return ClientError::new("API seems to have returned non-UTF8 garbage"); },
            };
            ClientError::new(format!("Failed to parse JSON: {}", string_body))
        })
    }
}

//! # teatime
//! A wrapper library for simplifying REST API implementations
//!
//! ## Motivation
//! When writing tools in Rust that talk with REST API endpoints,
//! there is often some boilerplate code needed when using `hyper`
//! for HTTP API requests. Much of this has to do with writing code
//! to handle futures and finding where some of the modification methods live
//! for things like headers and request body, something that is not always transparent to
//! programmers coming from an imperative language background. This
//! library abstracts away some of these details and allows the user
//! to either ignore the details of future handling
//! or optionally drop down to the future level, exposing a builder pattern
//! for HTTP requests for API flows.  Additionally helpful actions like
//! JSON API autopagination and HTTP body to JSON conversions are already implemented.
//!
//! ## Reference implementations
//! There are three reference implementations included, one for Sensu,
//! one for Gitlab, and one for Vault. This is probably the best example
//! of common patterns for defining to required methods that do not have
//! default implementations.
//!
//! ## Using teatime
//! The bulk of teatime is driven through the `HttpClient`, `ApiClient` and `JsonApiClient`
//! traits. There are additional data structures to help with type safety
//! when dealing with REST APIs that have a very loose type model.
//!
//! See the documentation for `ApiClient` and `JsonApiClient` as well as all of
//! data structures defined in `lib.rs` as these will outline parameter types,
//! return types and required implementation bits.
//!
//! ## Traditional request-response flows vs. future-based flows
//!
//! Once the `ApiClient` trait is implemented, an API can either be made through the `request`
//! method for a hyper `Response` type or `request_json` for automatic conversion of the 
//! response body to JSON.
//!
//! For cases where futures are desirable, there are two method calls for the request-response
//! flow. The first is `request_future`. This will return a future which can be left as is while
//! other work is done. This is the same for both `Response` and JSON flows. The resolution
//! functions that resolve to `Response`s and JSON objects are `response_future`
//! and `response_future_json` respectively.

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

use std::error::Error;
use std::fmt::{self,Formatter,Display};
use std::io::{self,Write};
use std::num;
use std::result;
use std::str;

use serde_json::{Value,Map};
use hyper::{Client,Method,Request,Response,Uri};
use hyper::client::{HttpConnector,FutureResponse};
use hyper::header::Header;
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Core;
use futures::{Future,Stream};

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

error_impl!(ClientError, serde_json::Error, hyper::Error, hyper::error::UriError,
            native_tls::Error, num::ParseIntError);

/// Result with `Error` type defined
pub type Result<T> = std::result::Result<T, ClientError>;

/// Prompt for text echoed in the terminal - _do not use for sensitive data_
pub fn interactive_text(prompt: &str) -> result::Result<String, io::Error> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    print!("{}", prompt);
    try!(stdout.flush());
    let mut line = String::new();
    let line_len = try!(stdin.read_line(&mut line));
    line.truncate(line_len - 1);

    Ok(line)
}

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

    /// Create a hyper `Request` object
    fn start_request(&mut self, Method, Uri) -> &mut Self;
    /// Add request headers
    fn add_header<H>(&mut self, H) -> &mut Self where H: Header;
    /// Set an individual header in the HTTP request
    fn add_body<S>(&mut self, S) -> &mut Self where S: ToString;
    /// Make HTTP request
    fn make_request(&mut self) -> &mut Self;
    /// Get complete HTTP response
    fn response(&mut self) -> Result<Response>;
    /// Get `Response` future
    fn future(&mut self) -> Option<FutureResponse>;
    /// Evaluate a future
    fn evaluate_future<F>(&mut self, future: F)
        -> result::Result<F::Item, F::Error> where F: Future;
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
    fn start_request(&mut self, method: Method, uri: Uri) -> &mut Self {
        self.request = Some(Request::new(method, uri));
        self
    }

    fn add_header<H>(&mut self, header: H) -> &mut Self where H: Header {
        self.request.as_mut().map(|ref mut req| req.headers_mut().set::<H>(header));
        self
    }

    fn add_body<S>(&mut self, body: S) -> &mut Self where S: ToString {
        self.request.as_mut().map(|ref mut req| req.set_body(body.to_string()));
        self
    }

    fn make_request(&mut self) -> &mut Self {
        let request = self.request.take();
        self.response_fut = request.map(|req| self.https_client.request(req));
        self
    }

    fn response(&mut self) -> Result<Response> {
        let response_fut = self.response_fut.take().ok_or(ClientError::new("No request made"))?;
        self.evaluate_future(response_fut).map_err(|e| {
            ClientError::new(e.description())
        })
    }

    fn future(&mut self) -> Option<FutureResponse> {
        self.response_fut.take()
    }

    fn evaluate_future<F>(&mut self, future: F) -> result::Result<F::Item, F::Error>
            where F: Future {
        self.core.run(future)
    }

}

/// Provides some default implementations for handling API level requests and flows
pub trait ApiClient<HTTP> where HTTP: ?Sized + HttpClient {
    /// Get base API URI to which all relative endpoint requests will be appended
    fn base_uri(&self) -> &Uri;
    /// Generate full URI for requests
    fn full_uri(&self, uri: Uri) -> Result<Uri> {
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
        Ok(full_uri)
    }
    /// Get underlying HTTP client
    fn http_client(&self) -> &HTTP;
    /// Get underlying HTTP client mutably
    fn http_client_mut(&mut self) -> &mut HTTP;
    /// Implement authentication here
    fn login(&mut self, &ApiCredentials) -> Result<()>;

    /// Make an API request and resolve the future to a response
    fn request<B>(&mut self, method: Method, uri: Uri, body: Option<B>) -> Result<Response>
            where B: ToString {
        let future = self.request_future(method, uri, body).ok_or(ClientError::new("No request made"))?;
        self.response_future(future)
    }
    /// Make an API request and return the future
    fn request_future<B>(&mut self, method: Method, uri: Uri, body: Option<B>) -> Option<FutureResponse> where B: ToString;
    /// Resolve the future to a response
    fn response_future(&mut self, f: FutureResponse) -> Result<Response> {
        Ok(self.http_client_mut().evaluate_future(f)?)
    }
}

/// Provides a default implementation for pagination in JSON API flows and automatic conversion from
/// response body to JSON
pub trait JsonApiClient<HTTP>: ApiClient<HTTP> where HTTP: HttpClient {
    /// Retrieves a URL for the request to get the next page in
    /// a paginated response
    fn next_page_uri<'a>(&mut self, resp: &Response)
                         -> Result<Option<Uri>>;

    /// Default implementation to make an API request and convert the response to JSON
    fn request_json<B>(&mut self, method: Method, uri: Uri,
                       body: Option<B>) -> Result<Value>
                       where B: ToString {
        let response = self.request(method, uri, body)?;
        self.response_to_json(response)
    }

    /// Resolve the future to a response and convert to JSON
    fn response_future_json(&mut self, fut: FutureResponse) -> Result<Value> {
        let response = self.response_future(fut)?;
        self.response_to_json(response)
    }

    /// Default implementation for handling pagination in JSON API contexts that will retrieve and
    /// parse all pages - *should not be used if page-by-page behavior is required*
    fn autopagination<B>(&mut self, method: Method, uri: Uri, body: Option<B>)
                         -> Result<Value> where B: ToString + Clone {
        let mut vec: Vec<Value> = Vec::new();
        let mut response = <Self as ApiClient<HTTP>>::request(
            self, method.clone(), uri.clone(), body.clone()
        )?;
        while let Some(page) = try!(self.next_page_uri(&response)) {
            let json = self.response_to_json(response)?;
            vec.push(json);
            response = <Self as ApiClient<HTTP>>::request(self, method.clone(), page, body.clone())?;
        }
        let json = self.response_to_json(response)?;
        vec.push(json);
        Ok(Value::Array(vec))
    }

    /// Convert a response body directly to JSON
    fn response_to_json(&mut self, response: Response) -> Result<Value> {
        let chunk = self.http_client_mut().evaluate_future(response.body().concat2())?;
        serde_json::from_slice(&chunk).map_err(|_e| {
            let string_body = match str::from_utf8(&chunk) {
                Ok(s) => s,
                _ => { return ClientError::new("API seems to have returned non-UTF8 garbage"); },
            };
            ClientError::new(format!("Failed to parse JSON: {}", string_body))
        })
    }
}

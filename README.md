# bma-jrpc

JSON-RPC Client for Rust

## Why yet another JSON RPC client when there are plenty?

The goal is to build a tiny client library which can construct a high or low
level RPC client with a couple of lines of code.

The library is perfect to write small code snippets for testing JSON-RPC
servers but can be also used in production applications as well.

## High-level client usage example

```rust
use bma_jrpc::{http_client, rpc_client};
use serde::Deserialize;
use std::time::Duration;

// Define a trait to map all RPC methods
#[rpc_client]
trait My {
    // the method returns null and has no params
    fn test(&self);
    // the method returns a structure
    fn login(&self, user: &str, password: &str) -> LoginResponse;
    // the method is mapped to RPC method "login"
    // it returns a structure but "result_field" attribute argument
    // automatically extracts "token" field only
    #[rpc(name = "login", result_field = "token")]
    fn authenticate(&self, user: &str, password: &str) -> String;
}

// The structure MyClient is automatically created for the above trait with a method "new"

// the response structure for the full "login" method output
#[derive(Deserialize, Debug)]
struct LoginResponse {
    api_version: u16,
    token: String,
}

// create a low-level HTTP RPC client
let http_client = http_client("http://localhost:7727").timeout(Duration::from_secs(2));
// create the high-level client
let client = MyClient::new(http_client);
let token = client.authenticate("admin", "xxx").unwrap();
dbg!(token);
let result: LoginResponse = client.login("admin", "xxx").unwrap();
dbg!(result);
```

## Low-level client usage example

```rust
use bma_jrpc::{http_client, Rpc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Deserialize, Debug)]
struct LoginResponse {
    api_version: u16,
    token: String,
}

#[derive(Serialize)]
struct LoginPayload<'a> {
    user: &'a str,
    password: &'a str,
}


// create a low-level HTTP RPC client
let http_client = http_client("http://localhost:7727").timeout(Duration::from_secs(2));
// use it directly. the params can be any which implements Serialize, the
// repsonse can be any which implements Deserialize
let result: LoginResponse = http_client.call(
    "login",
    LoginPayload {
        user: "admin",
        password: "xxx",
    },
).unwrap();
```

## MessagePack support

with "msgpack" feature an optional MessagePack RPC de/serialization can be enabled:

```rust
use bma_jrpc::{HttpClient, MsgPack};

// create a low-level HTTP RPC client
let http_client = HttpClient::<MsgPack>::new("http://localhost:7727");
// it can be used as a transport for high-level clients as well
// let client = MyClient::new(http_client);
```

## What is not supported (yet?)

* Bulk RPC requests

* RPC requests with no reply required (with no ID)

* Async in high-level clients

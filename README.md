# What is Garble?

Garble is a Rust implementation of the E2E peer-to-peer chat encrypted application. The users can input a host name and a port number to create a chatroom. Another user then is able to join the chatroom and chat through CLI.

## Installation and Building

To build the app, Rust and Git will be needed. The following steps contains the instruction on how to install Rust and run the code.

    1. Install Rust

        ```bash
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
        ```

    2. Clone the Garble repo

        ```bash
        git clone https://github.com/Exiled1/Garble.git
        ```

    3. Run the code
    
        ```bash
        cargo run
        ```

## Security Overview

Garble is built using the _Tokio_ asynchronous networking stack for client/server communcations and _OpenSSL_ for the starndard implementation of cryptographic primatives.

For the implementation of the cryptography scheme, the hybrid encryption is chosen for this task to ensure the large message between the users can be sent. The encryption is _AES256 GCM_ and _RSA2048_.

Use asymmetric encryption to generate a private and public key for the users. Use RSA key size of 2048, use fingerprint and exchange through a separate secured channel to prevent a middle man attack. One of the client is randomly chosen to generate the session key that will be used for the symmetric encryption for the chatroom. This session key will be encrypted with _RSA2048_ encryption and sign using the sender's private key, then it is shared to the other user, while the server cannot do anything to this encrypted key. The users can rely on the symmetric key encryption to be able to send messages back and forth without their messages being seen by anyone else. 
# Tests Certificates and Keys
keys and certificates generated using openssl and adapted from (webrtc-rs)[https://github.com/webrtc-rs/webrtc/tree/master/dtls/examples/certificates]

## generate commands
```shell
# Server.
$ SERVER_NAME='coap_server'
$ openssl ecparam -name prime256v1 -genkey -noout -out "${SERVER_NAME}.pem"
$ openssl req -key "${SERVER_NAME}.pem" -new -sha256 -subj '/C=NL' -out "${SERVER_NAME}.csr"
$ openssl x509 -req -in "${SERVER_NAME}.csr" -extfile "${EXTFILE}" -days 365 -signkey "${SERVER_NAME}.pem" -sha256 -out "${SERVER_NAME}.pub.pem"

# Client.
$ CLIENT_NAME='coap_client'
$ openssl ecparam -name prime256v1 -genkey -noout -out "${CLIENT_NAME}.pem"
$ openssl req -key "${CLIENT_NAME}.pem" -new -sha256 -subj '/C=NL' -out "${CLIENT_NAME}.csr"
$ openssl x509 -req -in "${CLIENT_NAME}.csr" -extfile "${EXTFILE}" -days 365 -CA "${SERVER_NAME}.pub.pem" -CAkey "${SERVER_NAME}.pem" -set_serial '0xabcd' -sha256 -out "${CLIENT_NAME}.pub.pem"
```

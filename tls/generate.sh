#!/usr/bin/env sh

set -ex

echo '{
  "signing": {
    "default": {
      "expiry": "87600h",
      "usages": [
        "signing",
        "key encipherment",
        "server auth",
        "client auth"
      ]
    }
  }
}' > cfssl.json

cfssl print-defaults csr | cfssl gencert -initca - | cfssljson -bare funtonic-ca

echo '{"key":{"algo":"rsa","size":2048}}' | cfssl gencert -ca=funtonic-ca.pem -ca-key=funtonic-ca-key.pem -config=cfssl.json \
    -hostname="funtonic.scoopit.io" - | cfssljson -bare server

echo '{"key":{"algo":"rsa","size":2048}}' | cfssl gencert -ca=funtonic-ca.pem -ca-key=funtonic-ca-key.pem -config=cfssl.json \
     - | cfssljson -bare executor

echo '{"key":{"algo":"rsa","size":2048}}' | cfssl gencert -ca=funtonic-ca.pem -ca-key=funtonic-ca-key.pem -profile=client \
    - | cfssljson -bare commander

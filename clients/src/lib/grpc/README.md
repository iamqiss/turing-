# gRPC / Connect

Auth and Messaging already expose tonic + tonic-web.
Generate TypeScript clients from `proto/` (or the out-of-tree CLIENT_CONTRACT) with `@bufbuild/protoc-gen-es` + `@connectrpc/protoc-gen-connect-es`.

Place generated code under `src/lib/grpc/gen/`.

//! gRPC + gRPC-Web service implementations.

pub mod pb {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("turing.tarpit.common.v1");
        }
    }
    pub mod auth {
        pub mod v1 {
            tonic::include_proto!("turing.tarpit.auth.v1");
        }
    }
    pub mod messaging {
        pub mod v1 {
            tonic::include_proto!("turing.tarpit.messaging.v1");
        }
    }
}

pub mod interceptors;
pub mod services;

pub use services::auth::AuthServiceImpl;
pub use services::messaging::MessagingServiceImpl;

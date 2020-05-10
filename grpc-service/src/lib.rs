//use thiserror::Error;

pub mod grpc_protocol;
pub mod payload;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");
/*
trait IntoGrpc {
    type Output;

    fn into_grpc(self) -> Self::Output;
}

trait FromGrpc<T> {
    //fn try_from(shitty_grpc_struct: T) -> Result<Self, GrpcConstraintError>;
}

#[derive(Debug, Error)]
enum GrpcConstraintError {
    #[error("The field `{0}` is mandatory")]
    MandatoryField(String),
}
*/

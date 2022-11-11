/// A succinct proof
pub trait Proof {
    type VerificationError: std::fmt::Debug;
    type MethodId;
    const MethodId: Self::MethodId;

    fn authenicated_log(&self) -> &[u8];
    fn verify(&self) -> Result<(), Self::VerificationError>;
}

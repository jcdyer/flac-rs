use crate::frame::Subframe;


#[derive(Default)]
pub struct Encoder {}

impl Encoder {
    pub fn encode_subframe_verbatim(subblock: &[u16]) -> Subframe<u16> {
        Subframe::Verbatim {
            value: subblock.to_owned(),
        }
    }
}

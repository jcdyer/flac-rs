use std::{convert::TryInto, ops::Not};

use crate::{
    frame::{ChannelLayout, Frame, Subblock, Subframe},
    headers::{BlockSize, MetadataBlockStreamInfo},
};

pub fn encode_subframe(subblock: &Subblock) -> Subframe<i16> {
    Subframe::encode_subblock(subblock).expect("can only handle i16 data")
}

pub enum Block {
    Stereo {
        left: Subblock,
        right: Subblock,
        mid: Subblock,
        side: Subblock,
    },
    Other {
        channels: Vec<Subblock>,
    },
}
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] // Need an arbitrary order to simplify stereo selection
enum ChannelKind {
    LeftRight,
    LeftSide,
    SideRight,
    MidSide,
}

impl Block {
    fn len(&self) -> usize {
        match self {
            Block::Stereo { left, .. } => left.len(),
            Block::Other { channels } => channels[0].len(),
        }
    }
    pub fn encode(
        &self,
        stream_info: &MetadataBlockStreamInfo,
        first_sample: u64,
    ) -> Option<Frame<i16>> {
        let mut frame = Frame::new(
            BlockSize::new(self.len().try_into().ok()?)?,
            stream_info,
            first_sample,
        )?;
        let layout = match self {
            Block::Stereo {
                left,
                right,
                mid,
                side,
            } => {
                // Select the best two channels to represent stereo
                let left_subframe = encode_subframe(left);
                let right_subframe = encode_subframe(right);
                ChannelLayout::Independent {
                    channels: vec![left_subframe, right_subframe],
                }
                /*
                // TODO: Side channel seems to misbehave with wrapping subtraction.  Maybe we need to use i32 for all inputs?
                //       The reference decoder seems to do that...

                let mid_subframe = encode_subframe(mid);
                let side_subframe = encode_subframe(side);

                let mut choices = [
                    (
                        left_subframe.len() + right_subframe.len(),
                        ChannelKind::LeftRight,
                    ),
                    (
                        mid_subframe.len() + side_subframe.len(),
                        ChannelKind::MidSide,
                    ),
                    (
                        left_subframe.len() + side_subframe.len(),
                        ChannelKind::LeftSide,
                    ),
                    (
                        side_subframe.len() + right_subframe.len(),
                        ChannelKind::SideRight,
                    ),
                ];
                choices.sort();
                let chosen_kind = choices[0].1;
                dbg!(
                    left_subframe.len(),
                    right_subframe.len(),
                    mid_subframe.len(),
                    side_subframe.len(),
                    chosen_kind
                );
                match chosen_kind {
                    ChannelKind::LeftRight => ChannelLayout::Independent {
                        channels: vec![left_subframe, right_subframe],
                    },
                    ChannelKind::LeftSide => ChannelLayout::LeftSide {
                        left: left_subframe,
                        side: side_subframe,
                    },
                    ChannelKind::SideRight => ChannelLayout::SideRight {
                        side: side_subframe,
                        right: right_subframe,
                    },
                    ChannelKind::MidSide => ChannelLayout::MidSide {
                        mid: mid_subframe,
                        side: side_subframe,
                    },
                }
                */
            }
            Block::Other { channels } => ChannelLayout::Independent {
                channels: channels.iter().map(encode_subframe).collect(),
            },
        };
        frame.set_subframes(layout);
        Some(frame)
    }

    pub fn from_input(mut channels: Vec<Subblock>) -> Block {
        assert!(channels.is_empty().not());
        assert!(channels.len() <= 8);
        if channels.len() == 2 {
            let mut drain = channels.drain(..);
            let left = drain.next().unwrap();
            let right = drain.next().unwrap();
            let (mid, side) = to_mid_side(&left, &right);
            Block::Stereo {
                left,
                right,
                mid,
                side,
            }
        } else {
            Block::Other { channels }
        }
    }
}

fn to_mid_side(left: &Subblock, right: &Subblock) -> (Subblock, Subblock) {
    assert_eq!(left.len(), right.len());
    match (left, right) {
        (Subblock::I16(left), Subblock::I16(right)) => {
            let (mvec, svec): (Vec<i16>, Vec<i16>) = left
                .iter()
                .zip(right)
                .map(|(l, r)| (((*l as i32 + *r as i32) / 2) as i16, l.wrapping_sub(*r)))
                .unzip();
            (Subblock::I16(mvec), Subblock::I16(svec))
        }

        _ => panic!("cannot calculate mid-side for subblocks of different variants"),
    }
}

/// An iterator to calculate residuals over
pub struct FixedResidual<'a, const ORDER: usize> {
    iter: std::iter::Copied<std::slice::Iter<'a, i16>>,
    residuals: [i32; ORDER],
}

impl<'a, const ORDER: usize> FixedResidual<'a, ORDER> {
    pub fn new(subblock: &'a [i16]) -> FixedResidual<'a, ORDER> {
        let mut iter = subblock.iter().copied();
        let mut residuals = [0; ORDER];
        for i in 0..ORDER {
            let mut prev = 0i32;
            let mut next = iter.next().unwrap() as i32;
            for residual in &mut residuals[..=i] {
                next -= prev;
                prev = *residual;
                *residual = next;
            }
        }

        FixedResidual { iter, residuals }
    }
}

impl<'a, const ORDER: usize> Iterator for FixedResidual<'a, ORDER> {
    type Item = i32;
    fn next(&mut self) -> Option<Self::Item> {
        let mut next = self.iter.next()? as i32;
        for residual in &mut self.residuals {
            let val = next;
            let residual_prev = *residual;
            *residual = val;
            next = val - residual_prev;
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Range, os::unix::thread};

    use rand::{thread_rng, Rng};

    use super::FixedResidual;
    #[test]
    fn order_zero() {
        for (slice, residual) in [
            (&[0, 1, 2, 4, 7][..], &[0, 1, 2, 4, 7][..]),
            (&[1, 2, 3, 4, 5, 6, 7], &[1, 2, 3, 4, 5, 6, 7]),
        ] {
            let fr = FixedResidual::<'_, 0>::new(slice);
            assert_eq!(&fr.into_iter().collect::<Vec<_>>(), residual);
        }
    }

    #[test]
    fn order_one() {
        for (slice, residual) in &[
            (&[0, 1, 2, 4, 7][..], &[1, 1, 2, 3][..]),
            (&[1, 2, 3, 4, 5, 6, 7], &[1, 1, 1, 1, 1, 1]),
            (
                &[1, 2, 3, 3, 2, 1, 1, 2, 3, 3, 2, 1],
                &[1, 1, 0, -1, -1, 0, 1, 1, 0, -1, -1],
            ),
        ] {
            let fr = FixedResidual::<'_, 1>::new(slice);
            let result = fr.into_iter().collect::<Vec<_>>();
            assert_eq!(slice.len() - 1, result.len());
            assert_eq!(&result, residual);
        }
    }

    #[test]
    fn order_two() {
        for (slice, residual) in &[
            (&[0, 1, 2, 4, 7][..], &[0, 1, 1][..]),
            (&[1, 2, 3, 4, 5, 6, 7], &[0, 0, 0, 0, 0]),
            (
                &[1, 2, 3, 3, 2, 1, 1, 2, 3, 3, 2, 1],
                &[0, -1, -1, 0, 1, 1, 0, -1, -1, 0],
            ),
        ] {
            let fr = FixedResidual::<'_, 2>::new(slice);
            let result = fr.into_iter().collect::<Vec<_>>();
            assert_eq!(slice.len() - 2, result.len());
            assert_eq!(&result, residual);
        }
    }

    #[test]
    fn order_three() {
        for (slice, residual) in &[
            (&[0, 1, 2, 4, 7][..], &[1, 0][..]),
            (&[1, 2, 3, 4, 5, 6, 7], &[0, 0, 0, 0]),
            (
                &[1, 2, 3, 3, 2, 1, 1, 2, 3, 3, 2, 1],
                &[-1, 0, 1, 1, 0, -1, -1, 0, 1],
            ),
        ] {
            let fr = FixedResidual::<'_, 3>::new(slice);
            let result = fr.into_iter().collect::<Vec<_>>();
            assert_eq!(slice.len() - 3, result.len());
            assert_eq!(&result, residual);
        }
    }

    #[test]
    fn overflow_and_underflow() {
        let slice = &[i16::MIN, i16::MAX, i16::MIN, i16::MAX][..];
        let mut fr = FixedResidual::<'_, 1>::new(slice);

        assert_eq!(fr.next(), Some(i16::MAX as i32 - i16::MIN as i32));
        assert_eq!(fr.next(), Some(-(i16::MAX as i32 - i16::MIN as i32)));

        let slice = &[i16::MIN, i16::MIN, i16::MAX][..];
        let mut fr = FixedResidual::<'_, 2>::new(slice);
        assert_eq!(fr.next(), Some(i16::MAX as i32 - i16::MIN as i32));
    }

    #[test]
    fn random_overflow_test() {
        let mut min2 = i32::MAX;
        let mut max2 = i32::MIN;

        let mut min3 = i32::MAX;
        let mut max3 = i32::MIN;
        for _ in 0..10000 {
            let arr: [i16; 12] = thread_rng().gen();
            let fr = FixedResidual::<'_, 2>::new(&arr[..]);
            for i in fr {
                if i < min2 {
                    min2 = i;
                }
                if i > max2 {
                    max2 = i;
                }
                assert!(
                    i <= 4 * i16::MAX as i32 + 1,
                    "ORDER 2 {:?} generated value {}",
                    arr,
                    i
                );
                assert!(
                    i >= 4 * i16::MIN as i32,
                    "ORDER 2 {:?} generated value {}",
                    arr,
                    i
                );
            }

            let fr = FixedResidual::<'_, 3>::new(&arr[..]);
            for i in fr {
                if i < min3 {
                    min3 = i;
                }
                if i > max3 {
                    max3 = i;
                }
                assert!(
                    i <= 8 * (i16::MAX as i32 + 1),
                    "ORDER 3 residual {:?} generated value {}",
                    arr,
                    i
                );

                assert!(
                    i >= 8 * i16::MIN as i32,
                    "ORDER 3 residual {:?} generated value {}",
                    arr,
                    i
                );
            }
        }
        assert!(dbg!(min2) < 3*i16::MIN as i32);
        assert!(dbg!(max2) > 3*i16::MAX as i32);
        assert!(dbg!(max3) > 7* i16::MAX as i32);
        assert!(dbg!(min3) < 7 * i16::MIN as i32);

        assert!(min2 >= 4*i16::MIN as i32);
        assert!(min3 >= 8*i16::MIN as i32);
        assert!(max2 <= 4*(1 + i16::MAX as i32));
        assert!(max3 <= 8*(1 + i16::MAX as i32));
    }
}

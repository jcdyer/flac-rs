use std::{convert::TryInto, ops::Not};

use crate::{
    frame::{ChannelLayout, Frame, Sample, Subblock, Subframe},
    headers::{BlockSize, MetadataBlockStreamInfo},
};

pub fn encode_subframe<S: Sample>(subblock: &Subblock<S>) -> Subframe<S> {
    Subframe::from_subblock(subblock)
}

pub enum Block<S: Sample> { // Side requires widened data
    Stereo {
        left: Subblock<S>,
        right: Subblock<S>,
        mid: Subblock<S>,
        side: Subblock<S::Widened>,
    },
    Other {
        channels: Vec<Subblock<S>>,
    },
}
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] // Need an arbitrary order to simplify stereo selection
enum ChannelKind {
    LeftRight,
    LeftSide,
    SideRight,
    MidSide,
}

impl<S: Sample> Block<S> {
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
    ) -> Option<Frame<S>> {
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
                let left_subframe =  Subframe::from_subblock(left);
                let right_subframe =  Subframe::from_subblock(right);
                let mid_subframe = Subframe::from_subblock(mid);
                match Subframe::<S>::encode_side_channel(side) {
                    None => ChannelLayout::Independent {
                        channels: vec![left_subframe, right_subframe],
                    },
                    Some(side_subframe) => {
                        let side_len = side_subframe.len();
                        let mut choices = [
                            (
                                left_subframe.len() + right_subframe.len(),
                                ChannelKind::LeftRight,
                            ),
                            (
                                mid_subframe.len() + side_len,
                                ChannelKind::MidSide,
                            ),
                            (
                                left_subframe.len() + side_len,
                                ChannelKind::LeftSide,
                            ),
                            (
                                side_len + right_subframe.len(),
                                ChannelKind::SideRight,
                            ),
                        ];
                        choices.sort();

                        // UNWRAP SAFETY: side_subframe is checked above.
                        let chosen_kind = choices[0].1;
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
                    }
                }
            }

            Block::Other { channels } => ChannelLayout::Independent {
                channels: channels.iter().map(encode_subframe).collect(),
            },
        };
        frame.set_subframes(layout);
        Some(frame)
    }

    pub fn from_input(mut channels: Vec<Subblock<S>>) -> Block<S> {
        assert!(channels.is_empty().not());
        assert!(channels.len() <= 8);
        if channels.len() == 2 {
            let mut channel_iter = channels.into_iter();
            let left = channel_iter.next().unwrap();
            let right = channel_iter.next().unwrap();
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

fn to_mid_side<S: Sample>(left: &Subblock<S>, right: &Subblock<S>) -> (Subblock<S>, Subblock<S::Widened>) {
    assert_eq!(left.len(), right.len());
    let (mvec, svec): (Vec<S>, Vec<S::Widened>) = left.data
        .iter()
        .zip(&right.data)
        .map(|(l, r)| (S::try_from_widened((l.widen() + r.widen()) >> 1).unwrap(), l.widen() - r.widen()))
        .unzip();
    (Subblock { data: mvec}, Subblock { data: svec })
}



/// An iterator to calculate residuals over
pub struct FixedResidual<'a, S, const ORDER: usize> {
    iter: std::iter::Copied<std::slice::Iter<'a, S>>,
    residuals: [i64; ORDER],
}

impl<'a, S, const ORDER: usize> FixedResidual<'a, S, ORDER>
where
    S: Sample,
{
    pub fn new(subblock: &'a [S]) -> FixedResidual<'a, S, ORDER> {
        let mut iter = subblock.iter().copied();
        let mut residuals = [0; ORDER];
        for i in 0..ORDER {
            let mut prev = 0;
            let mut next = iter.next().unwrap().to_i64();
            for residual in &mut residuals[..=i] {
                next -= prev;
                prev = *residual;
                *residual = next;
            }
        }

        FixedResidual { iter, residuals }
    }
}

impl<'a, S, const ORDER: usize> Iterator for FixedResidual<'a, S, ORDER>
where
    S: Sample,
{
    type Item = i64;
    fn next(&mut self) -> Option<Self::Item> {
        let mut next = self.iter.next()?.to_i64();
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
    use rand::{thread_rng, Rng};
    use super::FixedResidual;

    #[test]
    fn order_zero() {
        for (slice, residual) in [
            (&[0, 1, 2, 4, 7][..], &[0, 1, 2, 4, 7][..]),
            (&[1, 2, 3, 4, 5, 6, 7], &[1, 2, 3, 4, 5, 6, 7]),
        ] {
            let fr = FixedResidual::<'_, i16, 0>::new(slice);
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
            let fr = FixedResidual::<'_, i16, 1>::new(slice);
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
            let fr = FixedResidual::<'_, i64, 2>::new(slice);
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
            let fr = FixedResidual::<'_, i16, 3>::new(slice);
            let result = fr.into_iter().collect::<Vec<_>>();
            assert_eq!(slice.len() - 3, result.len());
            assert_eq!(&result, residual);
        }
    }

    #[test]
    fn overflow_and_underflow() {
        let slice = &[i16::MIN, i16::MAX, i16::MIN, i16::MAX][..];
        let mut fr = FixedResidual::<'_, i16, 1>::new(slice);

        assert_eq!(fr.next(), Some(i16::MAX as i64 - i16::MIN as i64));
        assert_eq!(fr.next(), Some(-(i16::MAX as i64 - i16::MIN as i64)));

        let slice = &[i16::MIN, i16::MIN, i16::MAX][..];
        let mut fr = FixedResidual::<'_, i16, 2>::new(slice);
        assert_eq!(fr.next(), Some(i16::MAX as i64 - i16::MIN as i64));
    }

    #[test]
    fn random_overflow_test() {
        let mut min2 = i64::MAX;
        let mut max2 = i64::MIN;

        let mut min3 = i64::MAX;
        let mut max3 = i64::MIN;

        for _ in 0..10000 {
            let arr: [i16; 12] = thread_rng().gen();
            let fr = FixedResidual::<'_, i16, 2>::new(&arr[..]);
            for i in fr {
                if i < min2 {
                    min2 = i;
                }
                if i > max2 {
                    max2 = i;
                }
                assert!(
                    i <= 4 * i16::MAX as i64 + 1,
                    "ORDER 2 {:?} generated value {}",
                    arr,
                    i
                );
                assert!(
                    i >= 4 * i16::MIN as i64,
                    "ORDER 2 {:?} generated value {}",
                    arr,
                    i
                );
            }

            let fr = FixedResidual::<'_, i16, 3>::new(&arr[..]);
            for i in fr {
                if i < min3 {
                    min3 = i;
                }
                if i > max3 {
                    max3 = i;
                }
                assert!(
                    i <= 8 * (i16::MAX as i64 + 1),
                    "ORDER 3 residual {:?} generated value {}",
                    arr,
                    i
                );

                assert!(
                    i >= 8 * i16::MIN as i64,
                    "ORDER 3 residual {:?} generated value {}",
                    arr,
                    i
                );
            }
        }
        assert!(dbg!(min2) < 3 * i16::MIN as i64);
        assert!(dbg!(max2) > 3 * i16::MAX as i64);
        assert!(dbg!(max3) > 7 * i16::MAX as i64);
        assert!(dbg!(min3) < 7 * i16::MIN as i64);

        assert!(min2 >= 4 * i16::MIN as i64);
        assert!(min3 >= 8 * i16::MIN as i64);
        assert!(max2 <= 4 * (1 + i16::MAX as i64));
        assert!(max3 <= 8 * (1 + i16::MAX as i64));
    }
}

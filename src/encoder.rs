use std::ops::Not;

use crate::frame::{Frame, Subblock, Subframe};

#[derive(Default)]
pub struct Encoder {
    block: Vec<Subblock>,
}

impl Encoder {
    pub fn encode_block(&self) -> Frame<i16> {
        todo!()
    }

    pub fn encode_subframe_verbatim(subblock: &[i16]) -> Subframe<i16> {
        Subframe::Verbatim {
            value: subblock.to_owned(),
        }
    }
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

impl Block {
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
                .into_iter()
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
    residuals: [i16; ORDER],
}

impl<'a, const ORDER: usize> FixedResidual<'a, ORDER> {
    fn new(subblock: &'a [i16]) -> FixedResidual<'a, ORDER> {
        let mut iter = subblock.iter().copied();
        let mut residuals = [0; ORDER];
        for i in 0..ORDER {
            let mut prev = 0;
            let mut next = iter.next().unwrap();
            for residual in &mut residuals[..=i] {
                next = next - prev;
                prev = *residual;
                *residual = next;
            }
        }

        println!("Initial residuals{:?}", residuals);
        FixedResidual { iter, residuals }
    }
}

impl<'a, const ORDER: usize> Iterator for FixedResidual<'a, ORDER> {
    type Item = i16;
    fn next(&mut self) -> Option<Self::Item> {
        let mut next = self.iter.next()?;
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
    use super::FixedResidual;
    #[test]
    fn order_zero() {
        for slice in [&[0, 1, 2, 4, 7][..], &[1, 2, 3, 4, 5, 6, 7]] {
            let fr = FixedResidual::<'_, 0>::new(slice);
            assert_eq!(&fr.into_iter().collect::<Vec<_>>(), slice);
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
}

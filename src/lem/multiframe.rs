use anyhow::Result;
use bellpepper::util_cs::witness_cs::WitnessCS;
use bellpepper_core::{num::AllocatedNum, Circuit, ConstraintSystem, SynthesisError};
use std::sync::Arc;

use crate::{
    circuit::gadgets::pointer::AllocatedPtr,
    coprocessor::Coprocessor,
    eval::lang::Lang,
    field::LurkField,
    proof::{MultiFrameTrait, Provable, FrameLike, Prover}, store,
};

use super::{
    circuit::{BoundAllocations, GlobalAllocator},
    interpreter::Frame,
    pointers::Ptr,
    store::Store,
    Func,
};

#[derive(Clone)]
pub struct MultiFrame<'a, F: LurkField, C: Coprocessor<F>> {
    pub store: Option<&'a Store<F>>,
    pub lang: Arc<Lang<F, C>>,
    pub func: Arc<Func>,
    pub input: Option<Vec<Ptr<F>>>,
    pub output: Option<Vec<Ptr<F>>>,
    pub frames: Option<Vec<Frame<F>>>,
    pub cached_witness: Option<WitnessCS<F>>,
    pub reduction_count: usize,
}

impl<F: LurkField> FrameLike for Frame<F> {
    type Ptr = Vec<Ptr<F>>;
    
    fn input(&self) -> &Self::Ptr {
        &self.input
    }
    fn output(&self) -> &Self::Ptr {
        &self.output
    }
}

impl<'a, F: LurkField, C: Coprocessor<F>> MultiFrameTrait<F, C> for MultiFrame<'a, F, C> {
    type Ptr = Ptr<F>;
    type Store = Store<F>;
    type StoreError = store::Error;
    type EvalFrame = Frame<F>;
    type CircuitFrame = Frame<F>;
    type GlobalAllocation = GlobalAllocator<F>;
    type AllocatedIO = Vec<AllocatedPtr<F>>;
    type FrameIter = <Self::FrameIntoIter as IntoIterator>::IntoIter;
    type FrameIntoIter = Vec<Self::CircuitFrame>;
    

    fn to_io_vector(store: &Self::Store, frames: &<Self::EvalFrame as FrameLike>::Ptr) -> Result<Vec<F>, Self::StoreError> {
        store.to_vector(frames).map_err(|e| store::Error(e.to_string()))
    }

    fn compute_witness(&self, s: &Store<F>) -> WitnessCS<F> {
        let mut wcs = WitnessCS::new();

        let z_scalar = s.to_vector(self.input.as_ref().unwrap()).unwrap();

        let mut bogus_cs = WitnessCS::<F>::new();
        let z: Vec<AllocatedNum<F>> = z_scalar
            .iter()
            .map(|x| AllocatedNum::alloc(&mut bogus_cs, || Ok(*x)).unwrap())
            .collect::<Vec<_>>();

        let _ = nova::traits::circuit::StepCircuit::synthesize(self, &mut wcs, z.as_slice());

        wcs
    }

    fn cached_witness(&mut self) -> &mut Option<WitnessCS<F>> {
        &mut self.cached_witness
    }

    fn frames(&self) -> Option<Self::FrameIntoIter> {
        self.frames
    }

    fn precedes(&self, maybe_next: &Self) -> bool {
        self.output == maybe_next.input
    }

    fn synthesize_frames<CS: ConstraintSystem<F>>(
        &self,
        cs: &mut CS,
        store: &Self::Store,
        input: Self::AllocatedIO,
        frames: &[Self::CircuitFrame],
        g: &Self::GlobalAllocation,
    ) -> Result<Self::AllocatedIO, SynthesisError> {
        let (_, output) = frames
            .iter()
            .try_fold((0, input.to_vec()), |(i, input), frame| {
                if !frame.blank {
                    for (alloc_ptr, input) in input.iter().zip(&frame.input) {
                        let input_zptr = store.hash_ptr(input).expect("Hash did not succeed");
                        match (alloc_ptr.tag().get_value(), alloc_ptr.hash().get_value()) {
                            (Some(alloc_ptr_tag), Some(alloc_ptr_hash)) => {
                                assert_eq!(alloc_ptr_tag, input_zptr.tag.to_field());
                                assert_eq!(alloc_ptr_hash, input_zptr.hash);
                            }
                            _ => return Err(SynthesisError::AssignmentMissing),
                        }
                    }
                }
                let bound_allocations = &mut BoundAllocations::new();
                self.func.add_input(&input, bound_allocations);
                let output = self
                    .func
                    .synthesize_frame(
                        &mut cs.namespace(|| format!("frame {i}")),
                        store,
                        frame,
                        g,
                        bound_allocations,
                    )
                    .expect("failed to synthesize frame");
                assert_eq!(input.len(), output.len());
                Ok((i + 1, output))
            })?;
        Ok(output)
    }

    fn blank(count: usize, lang: Arc<Lang<F, C>>) -> Self {
        Self {
            store: None,
            lang: lang.clone(),
            func: Func::from(&*lang).into(),
            input: None,
            output: None,
            frames: None,
            cached_witness: None,
            reduction_count: count,
        }
    }

    fn from_frames(
        count: usize,
        frames: &[Frame<F>],
        store: &Self::Store,
        lang: Arc<Lang<F, C>>,
    ) -> Vec<Self> {
        let total_frames = frames.len();
        let n = (total_frames + count - 1) / count;
        let mut multi_frames = Vec::with_capacity(n);
        let func = Arc::new(Func::from(&*lang));

        for chunk in frames.chunks(count) {
            let last_frame = chunk.last().expect("chunk must not be empty");
            let inner_frames = if chunk.len() < count {
                let mut inner_frames = Vec::with_capacity(count);
                inner_frames.extend(chunk.to_vec());
                inner_frames.resize(count, last_frame.clone());
                inner_frames
            } else {
                chunk.to_vec()
            };

            let output = last_frame.output.clone();
            let input = chunk[0].input.clone();

            let mf = MultiFrame {
                store: Some(store),
                lang: lang.clone(),
                func: func.clone(),
                input: Some(input),
                output: Some(output),
                frames: Some(inner_frames),
                cached_witness: None,
                reduction_count: count,
            };

            multi_frames.push(mf);
        }

        multi_frames
    }

    /// Make a dummy instance, duplicating `self`'s final `CircuitFrame`.
    fn make_dummy(
        count: usize,
        circuit_frame: Option<Self::CircuitFrame>,
        store: &Self::Store,
        lang: Arc<Lang<F, C>>,
    ) -> Self {
        let (frames, input, output) = if let Some(circuit_frame) = circuit_frame {
            (
                Some(vec![circuit_frame.clone(); count]),
                Some(circuit_frame.input),
                Some(circuit_frame.output),
            )
        } else {
            (None, None, None)
        };
        Self {
            store: Some(store),
            lang: lang.clone(),
            func: Func::from(&*lang).into(),
            input,
            output,
            frames,
            cached_witness: None,
            reduction_count: count,
        }
    }

    fn get_evaluation_frames(
        prover: &impl Prover<F, C, Self>,
        expr: Self::Ptr,
        env: Self::Ptr,
        store: &mut Self::Store,
        limit: usize,
        lang: &Lang<F, C>,
    ) -> std::result::Result<Vec<Self::EvalFrame>, crate::error::ProofError> {
        todo!()
    }
}

impl<'a, F: LurkField, C: Coprocessor<F>> Circuit<F> for MultiFrame<'a, F, C> {
    fn synthesize<CS: ConstraintSystem<F>>(self, cs: &mut CS) -> Result<(), SynthesisError> {
        let mut synth =
            |store: &Store<F>, frames: &[Frame<F>], input: &[Ptr<F>], output: &[Ptr<F>]| {
                let mut allocated_input = Vec::with_capacity(input.len());
                for (i, ptr) in input.iter().enumerate() {
                    let z_ptr = store.hash_ptr(ptr).expect("pointer hashing failed");

                    let allocated_tag = AllocatedNum::alloc(
                        &mut cs.namespace(|| format!("allocated tag for input {i}")),
                        || Ok(z_ptr.tag.to_field()),
                    )?;
                    allocated_tag
                        .inputize(&mut cs.namespace(|| format!("inputized tag for input {i}")))?;

                    let allocated_hash = AllocatedNum::alloc(
                        &mut cs.namespace(|| format!("allocated hash for input {i}")),
                        || Ok(z_ptr.hash),
                    )?;
                    allocated_hash
                        .inputize(&mut cs.namespace(|| format!("inputized hash for input {i}")))?;

                    allocated_input.push(AllocatedPtr::from_parts(allocated_tag, allocated_hash));
                }

                let mut allocated_output = Vec::with_capacity(output.len());
                for (i, ptr) in output.iter().enumerate() {
                    let z_ptr = store.hash_ptr(ptr).expect("pointer hashing failed");

                    let allocated_tag = AllocatedNum::alloc(
                        &mut cs.namespace(|| format!("allocated tag for output {i}")),
                        || Ok(z_ptr.tag.to_field()),
                    )?;
                    allocated_tag
                        .inputize(&mut cs.namespace(|| format!("inputized tag for output {i}")))?;

                    let allocated_hash = AllocatedNum::alloc(
                        &mut cs.namespace(|| format!("allocated hash for output {i}")),
                        || Ok(z_ptr.hash),
                    )?;
                    allocated_hash
                        .inputize(&mut cs.namespace(|| format!("inputized hash for output {i}")))?;

                    allocated_output.push(AllocatedPtr::from_parts(allocated_tag, allocated_hash));
                }

                let g = self.func.alloc_globals(cs, store)?;

                let allocated_output_result =
                    self.synthesize_frames(cs, store, allocated_input, frames, &g)?;

                assert_eq!(allocated_output.len(), allocated_output_result.len());

                for (i, (o_res, o)) in allocated_output_result
                    .iter()
                    .zip(allocated_output)
                    .enumerate()
                {
                    o_res.enforce_equal(
                        &mut cs.namespace(|| format!("outer output {i} is correct")),
                        &o,
                    );
                }

                Ok(())
            };

        let input = self
            .input
            .as_ref()
            .ok_or_else(|| SynthesisError::AssignmentMissing)?;
        let output = self
            .output
            .as_ref()
            .ok_or_else(|| SynthesisError::AssignmentMissing)?;

        match self.store {
            Some(store) => {
                let frames = self.frames.as_ref().unwrap();
                synth(store, frames, input, output)
            }
            None => {
                assert!(self.frames.is_none());
                let func = &self.func;
                let store = func.init_store();
                let blank_frame = Frame::blank(func);
                let frames = vec![blank_frame; self.reduction_count];
                synth(&store, &frames, input, output)
            }
        }
    }
}

impl<'a, F: LurkField, C: Coprocessor<F>> Provable<F> for MultiFrame<'a, F, C> {
    fn public_inputs(&self) -> Vec<F> {
        let input = self.input.as_ref().expect("input missing");
        let output = self.output.as_ref().expect("input missing");
        let store = self.store.expect("store missing");
        let mut res = Vec::with_capacity(self.public_input_size());
        for ptr in input {
            let z_ptr = store.hash_ptr(ptr).expect("pointer hashing failed");
            res.push(z_ptr.tag.to_field());
            res.push(z_ptr.hash);
        }
        for ptr in output {
            let z_ptr = store.hash_ptr(ptr).expect("pointer hashing failed");
            res.push(z_ptr.tag.to_field());
            res.push(z_ptr.hash);
        }
        res
    }

    #[inline]
    fn public_input_size(&self) -> usize {
        // tag and hash for input and output (output has the same size as the input)
        4 * self.func.input_params.len()
    }

    #[inline]
    fn reduction_count(&self) -> usize {
        self.reduction_count
    }
}

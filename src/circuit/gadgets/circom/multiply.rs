use bellperson::{gadgets::num::AllocatedNum, ConstraintSystem, SynthesisError};
use ff::PrimeField;
use nova_scotia::{calculate_witness, r1cs::CircomConfig, synthesize};

pub fn circom_multiply<F: PrimeField, CS: ConstraintSystem<F>>(
    cs: &mut CS,
    a: F,
    b: F,
    cfg: &CircomConfig<F>,
) -> Result<AllocatedNum<F>, SynthesisError> {
    let arg_in = ("arg_in".into(), vec![a, b]);
    let inputs = vec![arg_in];
    let witness = calculate_witness(cfg, inputs, true).expect("msg");

    synthesize(cs, cfg.r1cs.clone(), Some(witness))
}

#[cfg(test)]
mod tests {
    use nova_scotia::r1cs::CircomConfig;
    use pasta_curves::vesta::Scalar as Fr;
    use std::env::current_dir;

    use crate::circuit::gadgets::circom::multiply::circom_multiply;
    use bellperson::util_cs::test_cs::TestConstraintSystem;
    use bellperson::util_cs::Comparable;
    use bellperson::ConstraintSystem;

    #[test]
    fn circom_multiply_test() {
        // If file sha256/main.circom changes, run the following:
        // REMARK: the scalar field in Vesta curve is Pallas field.
        // Then the prime parameter must be pallas if you set Fr to vesta::Scalar.
        // circom main.circom --r1cs --wasm --sym --c --output . --prime pallas --json

        let mut root = current_dir().unwrap();
        root.push("src/circuit/gadgets/circom/multiply");
        let mut wtns = root.clone();
        wtns.push("main_js/main.wasm");
        let mut r1cs = root.clone();
        r1cs.push("main.r1cs");

        let mut cs = TestConstraintSystem::<Fr>::new();
        let mut cfg = CircomConfig::new(wtns, r1cs).unwrap();

        let output = circom_multiply(
            &mut cs.namespace(|| "circom_multiply"),
            Fr::from(5),
            Fr::from(7),
            &mut cfg,
        );

        let expected = Fr::from(35);
        assert!(output.is_ok());
        let output_num = output.unwrap();
        assert_eq!(output_num.get_value().unwrap(), expected);
        assert!(cs.is_satisfied());
        assert_eq!(1, cs.num_constraints());
        assert_eq!(1, cs.num_inputs());
        assert_eq!(3, cs.aux().len());
    }
}
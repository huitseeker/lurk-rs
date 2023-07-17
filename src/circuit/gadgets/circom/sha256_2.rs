use bellperson::{gadgets::num::AllocatedNum, ConstraintSystem, SynthesisError};
use ff::PrimeField;
use nova_scotia::reader::load_r1cs;
use nova_scotia::{synthesize, generate_witness_from_wasm};

#[allow(dead_code)]
pub fn sha256_circom<F: PrimeField, CS: ConstraintSystem<F>>(
    cs: &mut CS,
    input1: F,
    input2: F,
) -> Result<AllocatedNum<F>, SynthesisError> {
    let mut root = std::env::current_dir().unwrap();
    root.push("src/circuit/gadgets/circom/sha256");


    let circuit_file = root.join("main.r1cs");
    let r1cs = load_r1cs(&circuit_file);

    let witness_dir = root.join("main_js");
    let witness_output = root.join("output.wtns");
    let witness_input_json = format!("{{ \"arg_in\": [\"{}\", \"{}\"] }}", 0, 0);

    let witness = generate_witness_from_wasm(witness_dir, witness_input_json, witness_output);

    synthesize(cs, r1cs, Some(witness))
}

#[cfg(test)]
mod tests {
    use nova_scotia::r1cs::CircomConfig;
    use pasta_curves::vesta::Scalar as Fr;
    use std::env::current_dir;

    use crate::circuit::gadgets::circom::sha256::sha256_circom;
    use bellperson::util_cs::test_cs::TestConstraintSystem;
    use bellperson::util_cs::Comparable;
    use bellperson::ConstraintSystem;

    #[test]
    fn sha256_circom_test() {
        // If file sha256/main.circom changes, run the following:
        // REMARK: the scalar field in Vesta curve is Pallas field.
        // Then the prime parameter must be pallas if you set Fr to vesta::Scalar.
        // circom main.circom --r1cs --wasm --sym --c --output . --prime pallas --json

        let mut root = current_dir().unwrap();
        root.push("src/circuit/gadgets/circom/sha256");
        let mut wtns = root.clone();
        wtns.push("main_js/main.wasm");
        let mut r1cs = root.clone();
        r1cs.push("main.r1cs");

        let mut cs = TestConstraintSystem::<Fr>::new();
        let mut cfg = CircomConfig::new(wtns, r1cs).unwrap();

        let output = sha256_circom(
            &mut cs.namespace(|| "sha256_circom"),
            Fr::from(0),
            Fr::from(0),
            &mut cfg,
        );

        let expected = "0x00000000008619b3767c057fdf8e6d99fde2680c5d8517eb06761c0878d40c40";
        assert!(output.is_ok());
        let output_num = output.unwrap();
        assert!(format!("{:?}", output_num.get_value().unwrap()) == expected);
        assert!(cs.is_satisfied());
        assert_eq!(30134, cs.num_constraints());
        assert_eq!(1, cs.num_inputs());
        assert_eq!(29822, cs.aux().len());
    }
}
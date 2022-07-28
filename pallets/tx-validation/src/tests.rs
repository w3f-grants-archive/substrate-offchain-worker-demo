use crate::{mock::*, Error};
use frame_support::assert_ok;
use frame_support::pallet_prelude::ConstU32;
use frame_support::{assert_err, BoundedVec};

#[test]
fn store_metadata_ok() {
    new_test_ext().execute_with(|| {
        // Dispatch a signed extrinsic.
        let account_id = 1;
        let ipfs_cid = vec![1, 2];
        let message_digits = vec![3, 4];
        let pinpad_digits = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_ok!(TxValidation::store_metadata(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            message_digits.clone(),
            pinpad_digits.clone(),
        ));
        // Read pallet storage and assert an expected result.
        // MUST match the value [1,2] given to store_metadata
        let key_ipfs_hash: BoundedVec<u8, ConstU32<64>> = ipfs_cid.clone().try_into().unwrap();
        // MUST match the value [3,4] given to store_metadata
        let expected_message_digits: BoundedVec<u8, ConstU32<10>> =
            message_digits.clone().try_into().unwrap();
        let expected_pinpad_digits: BoundedVec<u8, ConstU32<10>> =
            pinpad_digits.clone().try_into().unwrap();
        let stored = TxValidation::circuit_server_metadata_map(account_id, key_ipfs_hash).unwrap();
        assert_eq!(stored.message_digits, expected_message_digits);
        assert_eq!(stored.pinpad_digits, expected_pinpad_digits);
    });
}

fn test_check_input_ok(inputs: Vec<u8>) {
    new_test_ext().execute_with(|| {
        let account_id = 1;
        let ipfs_cid = vec![1, 2];
        assert_ok!(TxValidation::store_metadata(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            // store_metadata is raw, as-is(no ascii conv)
            vec![3, 4],
            vec![4, 5, 6, 0, 1, 2, 3, 7, 8, 9],
        ));

        // Dispatch a signed extrinsic.
        assert_ok!(TxValidation::check_input(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            inputs
        ));
        System::assert_last_event(crate::Event::TxPass { account_id: 1 }.into());
    });
}

/// check_input SHOULD work with ASCII(useful for testing with a front-end)
#[test]
fn check_input_good_ascii_ok() {
    test_check_input_ok(vec!['6' as u8, '0' as u8])
}

#[test]
fn check_input_good_u8_ok() {
    test_check_input_ok(vec![6, 0])
}

#[test]
fn check_input_bad_error() {
    new_test_ext().execute_with(|| {
        let account_id = 1;
        let ipfs_cid = vec![1, 2];
        assert_ok!(TxValidation::store_metadata(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            // store_metadata is raw, as-is(no ascii conv)
            vec![3, 4],
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        ));

        // Dispatch a signed extrinsic.
        // Ensure the expected error is thrown if a wrong input is given
        let result = TxValidation::check_input(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            vec!['0' as u8, '0' as u8],
        );
        System::assert_last_event(crate::Event::TxFail { account_id: 1 }.into());
        assert_err!(result, Error::<Test>::TxWrongCodeGiven);
        // TODO? should this be a noop?
        // assert_noop!(
        //     TxValidation::check_input(Origin::signed(account_id), ipfs_cid.clone(), vec![0, 0]),
        //     Error::<Test>::TxWrongInputGiven
        // );
    });
}

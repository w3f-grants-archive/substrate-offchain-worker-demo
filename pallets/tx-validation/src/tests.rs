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
        let digits = vec![3, 4];
        assert_ok!(TxValidation::store_metadata(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            digits.clone()
        ));
        // Read pallet storage and assert an expected result.
        // MUST match the value [1,2] given to store_metadata
        let key_ipfs_hash: BoundedVec<u8, ConstU32<64>> = ipfs_cid.clone().try_into().unwrap();
        // MUST match the value [3,4] given to store_metadata
        let expected_value_digits: BoundedVec<u8, ConstU32<20>> =
            digits.clone().try_into().unwrap();
        let stored = TxValidation::circuit_server_metadata_map((account_id, key_ipfs_hash));
        assert_eq!(stored, (expected_value_digits,));
    });
}

#[test]
fn check_input_good_ok() {
    new_test_ext().execute_with(|| {
        let account_id = 1;
        let ipfs_cid = vec![1, 2];
        assert_ok!(TxValidation::store_metadata(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            // store_metadata is raw, as-is(no ascii conv)
            vec![3, 4]
        ));

        // Dispatch a signed extrinsic.
        assert_ok!(TxValidation::check_input(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            // check_input expects ASCII for now!
            vec!['3' as u8, '4' as u8]
        ));
        System::assert_last_event(crate::Event::TxPass { account_id: 1 }.into());
    });
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
            vec![3, 4]
        ));

        // Dispatch a signed extrinsic.
        // Ensure the expected error is thrown if a wrong input is given
        let result = TxValidation::check_input(
            Origin::signed(account_id),
            ipfs_cid.clone(),
            vec!['0' as u8, '0' as u8],
        );
        System::assert_last_event(crate::Event::TxFail { account_id: 1 }.into());
        assert_err!(result, Error::<Test>::TxWrongInputGiven);
        // TODO? should this be a noop?
        // assert_noop!(
        //     TxValidation::check_input(Origin::signed(account_id), ipfs_cid.clone(), vec![0, 0]),
        //     Error::<Test>::TxWrongInputGiven
        // );
    });
}

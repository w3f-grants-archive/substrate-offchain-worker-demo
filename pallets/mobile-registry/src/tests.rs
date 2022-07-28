use crate::{mock::*, Error};
use frame_support::assert_ok;
use frame_support::pallet_prelude::ConstU32;
use frame_support::{assert_err, assert_noop, BoundedVec};

fn test_register_mobile_ok(pub_key: Vec<u8>) {
    new_test_ext().execute_with(|| {
        let account_id = 1;

        // Dispatch a signed extrinsic.
        assert_ok!(MobileRegistry::register_mobile(
            Origin::signed(account_id),
            pub_key
        ));
        System::assert_last_event(crate::Event::NewMobileRegistered { account_id: 1 }.into());
    });
}

#[test]
fn test_register_mobile_basic_ok() {
    test_register_mobile_ok(vec![0; 32])
}

/// Android is apparently using a serialization(PKCS8?)
#[test]
fn test_register_mobile_android_ok() {
    test_register_mobile_ok(vec![
        48, 89, 48, 19, 6, 7, 42, 134, 72, 206, 61, 2, 1, 6, 8, 42, 134, 72, 206, 61, 3, 1, 7, 3,
        66, 0, 4, 186, 127, 99, 179, 251, 134, 228, 44, 51, 163, 15, 28, 47, 124, 216, 16, 163,
        184, 49, 5, 176, 21, 198, 136, 177, 67, 44, 197, 133, 149, 161, 38, 143, 218, 202, 47, 22,
        73, 105, 192, 57, 149, 67, 193, 58, 186, 113, 95, 159, 98, 62, 27, 185, 8, 164, 239, 118,
        140, 242, 135, 29, 221, 20, 250,
    ])
}

#[test]
fn test_register_mobile_pub_key_too_small_err() {
    new_test_ext().execute_with(|| {
        let account_id = 1;

        // Dispatch a signed extrinsic.
        // Ensure the expected error is thrown if a wrong input is given
        assert_noop!(
            MobileRegistry::register_mobile(Origin::signed(account_id), vec![0, 1]),
            Error::<Test>::InvalidKeySize
        );
    });
}

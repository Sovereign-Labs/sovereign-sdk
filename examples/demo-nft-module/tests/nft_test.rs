use demo_nft_module::{CallMessage, NonFungibleToken, UserAddress};
use sov_modules_api::{Context, Module};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};
use demo_nft_module::utils::get_collection_address;
use sov_modules_api::default_context::{DefaultContext};
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;


const PK1: [u8;32] = [199,23,116,41,227,173,69,178,7,24,164,151,88,149,52,187,102,167,163,248,38,86,207,66,87,81,56,66,211,150,208,155];
const PK2: [u8;32] = [92,136,187,3,235,27,9,215,232,93,24,78,85,255,234,60,152,21,139,246,151,129,152,227,231,204,38,84,159,129,71,143];
const PK3: [u8;32] = [233,139,68,72,169,252,229,117,72,144,47,191,13,42,32,107,190,52,102,210,161,208,245,116,93,84,37,87,171,44,30,239];

#[test]
fn mints_and_transfers() {
    let creator_pk = DefaultPrivateKey::try_from(&PK1[..]).unwrap();
    let private_key_1 = DefaultPrivateKey::try_from(&PK2[..]).unwrap();
    let private_key_2 = DefaultPrivateKey::try_from(&PK3[..]).unwrap();

    let creator_address = creator_pk.default_address();
    let collection_name = "Test Collection";
    let collection_uri = "http://foo.bar/test_collection";
    let collection_address = get_collection_address::<DefaultContext>(collection_name, creator_address.as_ref());

    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::<DefaultStorageSpec>::with_path(tmpdir.path()).unwrap());
    let nft = NonFungibleToken::default();

    let create_collection_message = CallMessage::CreateCollection {
        name: collection_name.to_string(),
        collection_uri: collection_uri.to_string()
    };

    let creator_context = DefaultContext::new(creator_address);

    // Create Collection
    nft.call(create_collection_message.clone(), &creator_context, &mut working_set)
        .expect("Creating Collection failed");

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();

    assert_eq!(actual_collection.name, collection_name);
    assert_eq!(actual_collection.supply, 0);
    assert_eq!(actual_collection.creator, UserAddress(creator_address));
    assert_eq!(actual_collection.frozen, false );

    let token_id = 42;
    let token_uri = "http://foo.bar/test_collection/42";
    let owner = UserAddress(private_key_1.default_address());

    let mint_nft_message = CallMessage::MintNft {
        collection_name: collection_name.to_string(),
        token_uri: token_uri.to_string(),
        token_id,
        owner: owner.clone(),
        frozen: false,
    };

    // Mint NFT to created collection
    nft.call(mint_nft_message.clone(), &creator_context, &mut working_set)
        .expect("Minting NFT failed");

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    assert_eq!(actual_collection.supply, 1);
    let actual_nft = nft.get_nft(collection_address.clone(),token_id, &mut working_set).unwrap();
    assert_eq!(actual_nft.token_id, token_id);
    assert_eq!(actual_nft.collection_address, collection_address.clone());
    assert_eq!(actual_nft.token_uri, token_uri.to_string());
    assert_eq!(actual_nft.owner, owner.clone());

    // Mint NFT to non-existent collection
    let ne_collection_name = "NON_EXISTENT_COLLECTION";
    let mint_nft_message = CallMessage::MintNft {
        collection_name: ne_collection_name.to_string(),
        token_uri: token_uri.to_string(),
        token_id,
        owner: owner.clone(),
        frozen: false,
    };
    let mint_response = nft.call(mint_nft_message.clone(), &creator_context, &mut working_set);
    if let Err(err) = mint_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "Collection with name {} by sender {} does not exist",
                    ne_collection_name,
                    creator_address.to_string()
                );
                assert_eq!(err_message, expected_message);
            },
        }
    } else {
        panic!("Expected an error, got Ok");
    }

    // Update a collection
    let new_collection_uri = "http://new/uri";
    let create_collection_message = CallMessage::UpdateCollection {
        name: collection_name.to_string(),
        collection_uri: new_collection_uri.to_string()
    };

    let creator_context = DefaultContext::new(creator_address);

    nft.call(create_collection_message.clone(), &creator_context, &mut working_set)
        .expect("Updating Collection failed");

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    assert_eq!(actual_collection.collection_uri, new_collection_uri.to_string());
    assert_eq!(actual_collection.frozen, false);

    // Freeze a non existent collection
    let freeze_collection_message = CallMessage::FreezeCollection {
        collection_name: ne_collection_name.to_string(),
    };

    let creator_context = DefaultContext::new(creator_address);

    let freeze_response = nft.call(freeze_collection_message.clone(), &creator_context, &mut working_set);
    if let Err(err) = freeze_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "Collection with name {} by sender {} does not exist",
                    ne_collection_name,
                    creator_address.to_string()
                );
                assert_eq!(err_message, expected_message);
            },
        }
    } else {
        panic!("Expected an error, got Ok");
    }

    // Freeze collection
    let freeze_collection_message = CallMessage::FreezeCollection {
        collection_name: collection_name.to_string(),
    };

    let creator_context = DefaultContext::new(creator_address);
    nft.call(freeze_collection_message.clone(), &creator_context, &mut working_set)
        .expect("Freeze collection call should succeed");

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    assert_eq!(actual_collection.frozen, true);

    // Update collection uri for frozen collection
    // Update a collection
    let un_updated_collection_uri = "http://new/uri2";
    let create_collection_message = CallMessage::UpdateCollection {
        name: collection_name.to_string(),
        collection_uri: un_updated_collection_uri.to_string()
    };

    let creator_context = DefaultContext::new(creator_address);

    let update_response = nft.call(create_collection_message.clone(), &creator_context, &mut working_set);
    if let Err(err) = update_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "Collection with name {} by sender {} is frozen and cannot be updated",
                    collection_name,
                    creator_address.to_string()
                );
                assert_eq!(err_message, expected_message);
            },
        }
    } else {
        panic!("Expected an error, got Ok");
    }

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    assert_eq!(actual_collection.frozen, true);
    // assert that the collection uri hasn't been changed
    assert_eq!(actual_collection.collection_uri, new_collection_uri);
    // assert that supply hasn't been modified
    assert_eq!(actual_collection.supply, 1);

    // mint nft to frozen collection
    let new_token_id = 23;
    let new_token_uri = "http://foo.bar/test_collection/23";
    let owner = UserAddress(private_key_1.default_address());

    let mint_nft_message = CallMessage::MintNft {
        collection_name: collection_name.to_string(),
        token_uri: new_token_uri.to_string(),
        token_id: new_token_id,
        owner: owner.clone(),
        frozen: false,
    };

    let mint_response = nft.call(mint_nft_message.clone(), &creator_context, &mut working_set);
    if let Err(err) = mint_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "Collection with name {} by sender {} is already frozen",
                    collection_name,
                    creator_address.to_string()
                );
                assert_eq!(err_message, expected_message);
            },
        }
    } else {
        panic!("Expected an error, got Ok");
    }

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    // ensure supply hasn't changed
    assert_eq!(actual_collection.supply, 1);


    // transfer NFT with non-owner
    let target_address = private_key_2.default_address();
    let transfer_nft_message = CallMessage::TransferNft {
        collection_address: collection_address.clone(),
        token_id,
        to: UserAddress(target_address),
    };

    // calling with the old context first (which is the creator)
    // but the creator is not the owner, so it should fail
    let transfer_response = nft.call(transfer_nft_message.clone(), &creator_context, &mut working_set);
    if let Err(err) = transfer_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "Transfer sent with owner {}, NFT id {} in Collection with address {} is owned by {}",
                    creator_address,
                    token_id,
                    &collection_address.0,
                    &owner.0
                );
                assert_eq!(err_message, expected_message);
            },
        }
    } else {
        panic!("Expected an error, got Ok");
    }

    // transfer NFT with non-existent token id
    let target_address = private_key_2.default_address();
    let owner_context = DefaultContext::new(owner.0.clone());
    let transfer_nft_message = CallMessage::TransferNft {
        collection_address: collection_address.clone(),
        token_id: 1000,
        to: UserAddress(target_address),
    };

    let transfer_response = nft.call(transfer_nft_message.clone(), &owner_context, &mut working_set);
    if let Err(err) = transfer_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "NFT id {} in Collection with address {} does not exist",
                    1000,
                    collection_address.0.clone()
                );
                assert_eq!(err_message, expected_message);
            },
        }
    } else {
        panic!("Expected an error, got Ok");
    }


    // transfer NFT by owner
    let target_address = private_key_2.default_address();
    let owner_context = DefaultContext::new(owner.0.clone());
    let transfer_nft_message = CallMessage::TransferNft {
        collection_address: collection_address.clone(),
        token_id,
        to: UserAddress(target_address),
    };
    let transfer_response = nft.call(transfer_nft_message.clone(), &owner_context, &mut working_set);
    assert!(transfer_response.is_ok());

    let actual_nft = nft.get_nft(collection_address.clone(),token_id, &mut working_set).unwrap();
    // ensure token_id didn't change
    assert_eq!(actual_nft.token_id, token_id);
    assert_eq!(actual_nft.collection_address, collection_address.clone());
    assert_eq!(actual_nft.token_uri, token_uri.to_string());
    // ensure that the owner is the new owner
    assert_eq!(actual_nft.owner, UserAddress(target_address.clone()));

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    // ensure supply hasn't changed with a transfer
    assert_eq!(actual_collection.supply, 1);

    // Update NFT token_uri
    let token_id = 42;
    let new_token_uri = "http://foo.bar/test_collection/new_url/42";
    let update_nft_message = CallMessage::UpdateNft {
        collection_address: collection_address.clone(),
        token_id,
        token_uri: Some(new_token_uri.to_string()),
        frozen: None,
    };
    let update_response = nft.call(update_nft_message.clone(), &creator_context, &mut working_set);
    assert!(update_response.is_ok());

    let actual_nft = nft.get_nft(collection_address.clone(),token_id, &mut working_set).unwrap();
    // ensure token_id didn't change
    assert_eq!(actual_nft.token_id, token_id);
    assert_eq!(actual_nft.collection_address, collection_address.clone());
    // token uri should be updated
    assert_eq!(actual_nft.token_uri, new_token_uri.to_string());
    // ensure owner is unchanged (new based on previous test)
    assert_eq!(actual_nft.owner, UserAddress(target_address.clone()));
    // ensure still unfrozen
    assert_eq!(actual_nft.frozen, false);

    // Freeze NFT
    let token_id = 42;
    let update_nft_message = CallMessage::UpdateNft {
        collection_address: collection_address.clone(),
        token_id,
        token_uri: None,
        frozen: Some(true),
    };
    let update_response = nft.call(update_nft_message.clone(), &creator_context, &mut working_set);
    assert!(update_response.is_ok());

    let actual_nft = nft.get_nft(collection_address.clone(),token_id, &mut working_set).unwrap();
    // ensure token_id didn't change
    assert_eq!(actual_nft.token_id, token_id);
    assert_eq!(actual_nft.collection_address, collection_address.clone());
    // token uri should be updated
    assert_eq!(actual_nft.token_uri, new_token_uri.to_string());
    // ensure owner is unchanged (new based on previous test)
    assert_eq!(actual_nft.owner, UserAddress(target_address.clone()));
    // ensure frozen is true
    assert_eq!(actual_nft.frozen, true);

    // Update NFT token uri for frozen NFT
    let token_id = 42;
    let new_token_uri_fail = "http://foo.bar/test_collection/new_url_fail/42";
    let update_nft_message = CallMessage::UpdateNft {
        collection_address: collection_address.clone(),
        token_id,
        token_uri: Some(new_token_uri_fail.to_string()),
        frozen: None,
    };
    let update_response = nft.call(update_nft_message.clone(), &creator_context, &mut working_set);
    if let Err(err) = update_response {
        match err {
            sov_modules_api::Error::ModuleError(anyhow_err) => {
                let err_message = anyhow_err.to_string();
                let expected_message = format!(
                    "NFT id {} in Collection with address {} is frozen",
                    token_id,
                    collection_address.0.clone()
                );
                assert_eq!(err_message, expected_message);
            }
        }
    } else {
        panic!("Expected an error, got Ok");
    }

    // ensure that token uri is unchanged
    let actual_nft = nft.get_nft(collection_address.clone(),token_id, &mut working_set).unwrap();
    // token uri should be unchanged. it should be new_token_uri, not new_token_uri_fail
    assert_eq!(actual_nft.token_uri, new_token_uri.to_string());

    // Transfer on a frozen NFT should still work
    // transfer NFT by owner
    let target_address = private_key_1.default_address();
    let owner = private_key_2.default_address();
    let owner_context = DefaultContext::new(owner.clone());
    let transfer_nft_message = CallMessage::TransferNft {
        collection_address: collection_address.clone(),
        token_id,
        to: UserAddress(target_address),
    };
    let transfer_response = nft.call(transfer_nft_message.clone(), &owner_context, &mut working_set);
    assert!(transfer_response.is_ok());

    let actual_nft = nft.get_nft(collection_address.clone(),token_id, &mut working_set).unwrap();
    // ensure token_id didn't change
    assert_eq!(actual_nft.token_id, token_id);
    assert_eq!(actual_nft.collection_address, collection_address.clone());
    // token uri should be new_token_uri
    assert_eq!(actual_nft.token_uri, new_token_uri.to_string());
    // ensure that the owner is the new owner
    assert_eq!(actual_nft.owner, UserAddress(target_address.clone()));

    let actual_collection = nft.get_collection(collection_address.clone(), &mut working_set).unwrap();
    // ensure supply hasn't changed with a transfer
    assert_eq!(actual_collection.supply, 1);

}


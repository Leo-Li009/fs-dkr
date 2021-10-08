#[cfg(test)]
mod tests {
    use crate::refresh_message::RefreshMessage;
    use curv::arithmetic::Converter;
    use curv::cryptographic_primitives::hashing::hash_sha256::HSha256;
    use curv::cryptographic_primitives::hashing::traits::Hash;
    use curv::cryptographic_primitives::secret_sharing::feldman_vss::{
        ShamirSecretSharing, VerifiableSS,
    };
    use curv::elliptic::curves::secp256_k1::GE;
    use curv::BigInt;
    use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2020::party_i::verify;
    use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2020::state_machine::keygen::{
        Keygen, LocalKey,
    };
    use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2020::state_machine::sign::{
        CompletedOfflineStage, OfflineStage, SignManual,
    };

    use crate::add_party::JoinMessage;
    use crate::error::FsDkrResult;
    use paillier::DecryptionKey;
    use round_based::dev::Simulation;
    use std::collections::HashMap;

    #[test]
    fn test1() {
        //simulate keygen
        let t = 3;
        let n = 5;
        let mut keys = simulate_keygen(t, n);

        let old_keys = keys.clone();
        simulate_dkr(&mut keys);

        // check that sum of old keys is equal to sum of new keys
        let old_linear_secret_key: Vec<_> = (0..old_keys.len())
            .map(|i| old_keys[i].keys_linear.x_i)
            .collect();

        let new_linear_secret_key: Vec<_> =
            (0..keys.len()).map(|i| keys[i].keys_linear.x_i).collect();
        let indices: Vec<_> = (0..(t + 1) as usize).collect();
        let vss = VerifiableSS::<GE> {
            parameters: ShamirSecretSharing {
                threshold: t as usize,
                share_count: n as usize,
            },
            commitments: Vec::new(),
        };
        assert_eq!(
            vss.reconstruct(&indices[..], &old_linear_secret_key[0..(t + 1) as usize]),
            vss.reconstruct(&indices[..], &new_linear_secret_key[0..(t + 1) as usize])
        );
        assert_ne!(old_linear_secret_key, new_linear_secret_key);
    }

    #[test]
    fn test_sign_rotate_sign() {
        let mut keys = simulate_keygen(2, 5);
        let offline_sign = simulate_offline_stage(keys.clone(), &[1, 2, 3]);
        simulate_signing(offline_sign, b"ZenGo");
        simulate_dkr(&mut keys);
        let offline_sign = simulate_offline_stage(keys.clone(), &[2, 3, 4]);
        simulate_signing(offline_sign, b"ZenGo");
        simulate_dkr(&mut keys);
        let offline_sign = simulate_offline_stage(keys, &[1, 3, 5]);
        simulate_signing(offline_sign, b"ZenGo");
    }

    #[test]
    fn test_remove_sign_rotate_sign() {
        let mut keys = simulate_keygen(2, 5);
        let offline_sign = simulate_offline_stage(keys.clone(), &[1, 2, 3]);
        simulate_signing(offline_sign, b"ZenGo");
        simulate_dkr_removal(&mut keys, [1].to_vec());
        let offline_sign = simulate_offline_stage(keys.clone(), &[2, 3, 4]);
        simulate_signing(offline_sign, b"ZenGo");
        simulate_dkr_removal(&mut keys, [1, 2].to_vec());
        let offline_sign = simulate_offline_stage(keys, &[3, 4, 5]);
        simulate_signing(offline_sign, b"ZenGo");
    }

    #[test]
    fn simulate_dkr_add() {
        fn simulate_addition(
            keys: &mut Vec<LocalKey>,
            party_indices: &[usize],
            t: usize,
            n: usize,
        ) -> FsDkrResult<()> {
            // TODO: introduce voting for party_index, now we hardcode it.

            // the new party generates it's broadcast message to start joining the computation
            let mut join_messages: Vec<_> = Vec::new();

            for party_index in party_indices.iter() {
                let (mut new_party_refresh_message, key) = JoinMessage::distribute();
                new_party_refresh_message.party_index = Some(*party_index);
                join_messages.push((new_party_refresh_message, key))
            }

            let new_parties_refresh_messages: Vec<JoinMessage> =
                join_messages.iter().map(|elem| elem.0.clone()).collect();

            let (broadcast_vec, new_dks): (Vec<_>, Vec<_>) = keys
                .iter_mut()
                .map(|key| {
                    let _results: Vec<_> = new_parties_refresh_messages
                        .iter()
                        .map(|new_party| {
                            key.paillier_key_vec[new_party.party_index.unwrap() - 1] =
                                new_party.ek.clone();
                            key.h1_h2_n_tilde_vec[new_party.party_index.unwrap() - 1] =
                                new_party.dlog_statement.clone();
                        })
                        .collect();
                    RefreshMessage::distribute(key)
                })
                .unzip();

            // all the other parties will receive it's dummy "refresh message" that signals that a party wants to join.
            // keys will be updated to refreshed values
            for i in 0..keys.len() as usize {
                RefreshMessage::collect(
                    &broadcast_vec,
                    &mut keys[i],
                    new_dks[i].clone(),
                    new_parties_refresh_messages.as_slice(),
                )
                .expect("");
            }

            // if not enough parties trust this new party, the t and n constraints will not be satisfied
            // and the new party will not be able to collect
            for (join_message, dk) in join_messages {
                let party_index = join_message.party_index.unwrap();
                let mut local_key =
                    JoinMessage::collect(broadcast_vec.as_slice(), dk, party_index, t, n)?;
                local_key.h1_h2_n_tilde_vec = keys[0].h1_h2_n_tilde_vec.clone();
                keys.insert(party_index - 1, local_key);
            }

            Ok(())
        }

        let t = 2;
        let n = 6;

        let all_keys = simulate_keygen(t, n);
        let mut keys = all_keys[0..5].to_vec();

        let offline_sign = simulate_offline_stage(keys.clone(), &[1, 2, 3]);
        simulate_signing(offline_sign, b"ZenGo");

        simulate_dkr(&mut keys);
        let offline_sign = simulate_offline_stage(keys.clone(), &[2, 3, 4]);
        simulate_signing(offline_sign, b"ZenGo");

        simulate_addition(&mut keys, &[6], t as usize, n as usize).unwrap();
        let offline_sign = simulate_offline_stage(keys.clone(), &[2, 3, 4]);
        simulate_signing(offline_sign, b"ZenGo");

        simulate_dkr(&mut keys);
        let offline_sign = simulate_offline_stage(keys, &[1, 4, 6]);
        simulate_signing(offline_sign, b"ZenGo");
    }

    fn simulate_keygen(t: u16, n: u16) -> Vec<LocalKey> {
        //simulate keygen
        let mut simulation = Simulation::new();
        simulation.enable_benchmarks(false);

        for i in 1..=n {
            simulation.add_party(Keygen::new(i, t, n).unwrap());
        }

        simulation.run().unwrap()
    }

    fn simulate_dkr_removal(keys: &mut Vec<LocalKey>, remove_party_indices: Vec<usize>) {
        let mut broadcast_messages: HashMap<usize, Vec<RefreshMessage<GE>>> = HashMap::new();
        let mut new_dks: HashMap<usize, DecryptionKey> = HashMap::new();
        let mut refresh_messages: Vec<RefreshMessage<GE>> = Vec::new();
        let mut party_key: HashMap<usize, LocalKey> = HashMap::new();

        for key in keys.iter_mut() {
            let (refresh_message, new_dk) = RefreshMessage::distribute(key);
            refresh_messages.push(refresh_message.clone());
            new_dks.insert(refresh_message.party_index, new_dk);
            party_key.insert(refresh_message.party_index, key.clone());
        }

        for refresh_message in refresh_messages.iter() {
            broadcast_messages.insert(refresh_message.party_index, Vec::new());
        }

        for refresh_message in refresh_messages.iter_mut() {
            if !remove_party_indices.contains(&refresh_message.party_index) {
                refresh_message.remove_party_indices = remove_party_indices.clone();
            } else {
                let mut new_remove_party_indices = remove_party_indices.clone();
                new_remove_party_indices.retain(|value| *value != refresh_message.party_index);
                refresh_message.remove_party_indices = new_remove_party_indices;
            }

            for (party_index, refresh_bucket) in broadcast_messages.iter_mut() {
                if refresh_message.remove_party_indices.contains(party_index) {
                    continue;
                }
                refresh_bucket.push(refresh_message.clone());
            }
        }

        for remove_party_index in remove_party_indices.iter() {
            assert_eq!(broadcast_messages[remove_party_index].len(), 1);
        }

        // keys will be updated to refreshed values
        for (party, key) in party_key.iter_mut() {
            if remove_party_indices.contains(party) {
                continue;
            }

            RefreshMessage::collect(
                broadcast_messages[party].clone().as_slice(),
                key,
                new_dks[party].clone(),
                &[],
            )
            .expect("");
        }

        for remove_party_index in remove_party_indices {
            let result = RefreshMessage::collect(
                &broadcast_messages[&remove_party_index],
                &mut keys[remove_party_index],
                new_dks[&remove_party_index].clone(),
                &[],
            );
            assert!(result.is_err());
        }
    }

    fn simulate_dkr(keys: &mut Vec<LocalKey>) -> (Vec<RefreshMessage<GE>>, Vec<DecryptionKey>) {
        let mut broadcast_vec: Vec<RefreshMessage<GE>> = Vec::new();
        let mut new_dks: Vec<DecryptionKey> = Vec::new();

        for key in keys.iter() {
            let (refresh_message, new_dk) = RefreshMessage::distribute(key);
            broadcast_vec.push(refresh_message);
            new_dks.push(new_dk);
        }

        // keys will be updated to refreshed values
        for i in 0..keys.len() as usize {
            RefreshMessage::collect(&broadcast_vec, &mut keys[i], new_dks[i].clone(), &[])
                .expect("");
        }

        (broadcast_vec, new_dks)
    }

    fn simulate_offline_stage(
        local_keys: Vec<LocalKey>,
        s_l: &[u16],
    ) -> Vec<CompletedOfflineStage> {
        let mut simulation = Simulation::new();
        simulation.enable_benchmarks(false);

        for (i, &keygen_i) in (1..).zip(s_l) {
            simulation.add_party(
                OfflineStage::new(
                    i,
                    s_l.to_vec(),
                    local_keys[usize::from(keygen_i - 1)].clone(),
                )
                .unwrap(),
            );
        }

        simulation.run().unwrap()
    }

    fn simulate_signing(offline: Vec<CompletedOfflineStage>, message: &[u8]) {
        let message = HSha256::create_hash(&[&BigInt::from_bytes(message)]);
        let pk = *offline[0].public_key();

        let parties = offline
            .iter()
            .map(|o| SignManual::new(message.clone(), o.clone()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let (parties, local_sigs): (Vec<_>, Vec<_>) = parties.into_iter().unzip();
        // parties.remove(0).complete(&local_sigs[1..]).unwrap();
        let local_sigs_except = |i: usize| {
            let mut v = vec![];
            v.extend_from_slice(&local_sigs[..i]);
            if i + 1 < local_sigs.len() {
                v.extend_from_slice(&local_sigs[i + 1..]);
            }
            v
        };

        assert!(parties
            .into_iter()
            .enumerate()
            .map(|(i, p)| p.complete(&local_sigs_except(i)).unwrap())
            .all(|signature| verify(&signature, &pk, &message).is_ok()));
    }
}

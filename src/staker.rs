// import { BigintIsh, Token, validateAndParseAddress } from '@uniswap/sdk-core'
// import { MethodParameters, toHex } from './utils/calldata'
// import { defaultAbiCoder, Interface } from '@ethersproject/abi'
// import IUniswapV3Staker from '@uniswap/v3-staker/artifacts/contracts/UniswapV3Staker.sol/UniswapV3Staker.json'
// import { Pool } from './entities'
// import { Multicall } from './multicall'
use crate::prelude::*;
use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolCall, SolValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullWithdrawOptions {
    pub claim_options: ClaimOptions,
    pub withdraw_options: WithdrawOptions,
}

/// Options to specify when claiming rewards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimOptions {
    /// The id of the NFT
    pub token_id: U256,
    /// Address to send rewards to.
    pub recipient: Address,
    /// The amount of `reward_token` to claim. 0 claims all.
    pub amount: Option<U256>,
}

/// Options to specify when withdrawing a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawOptions {
    /// Set when withdrawing. The position will be sent to `owner` on withdraw.
    pub owner: Address,
    /// Set when withdrawing. `data` is passed to `safeTransferFrom` when transferring the position from contract back to owner.
    pub data: Option<Vec<u8>>,
}

/// Represents a unique staking program.
#[derive(Debug, Clone, PartialEq)]
pub struct IncentiveKey<P> {
    /// The token rewarded for participating in the staking program.
    pub reward_token: Address,
    /// The pool that the staked positions must provide in.
    pub pool: Pool<P>,
    /// The time when the incentive program begins.
    pub start_time: U256,
    /// The time that the incentive program ends.
    pub end_time: U256,
    /// The address which receives any remaining reward tokens at `end_time`.
    pub refundee: Address,
}

fn encode_incentive_key<P>(incentive_key: &IncentiveKey<P>) -> IUniswapV3Staker::IncentiveKey {
    IUniswapV3Staker::IncentiveKey {
        rewardToken: incentive_key.reward_token,
        pool: incentive_key.pool.address(None, None),
        startTime: incentive_key.start_time,
        endTime: incentive_key.end_time,
        refundee: incentive_key.refundee,
    }
}

/// To claim rewards, must unstake and then claim.
///
/// ## Arguments
///
/// * `incentive_key`: The unique identifier of a staking program.
/// * `options`: Options for producing the calldata to claim. Can't claim unless you unstake.
///
/// ## Returns
///
/// The calldatas for 'unstakeToken' and 'claimReward'.
///
fn encode_claim<P>(incentive_key: &IncentiveKey<P>, options: ClaimOptions) -> [Vec<u8>; 2] {
    [
        IUniswapV3Staker::unstakeTokenCall {
            key: encode_incentive_key(incentive_key),
            tokenId: options.token_id,
        }
        .abi_encode(),
        IUniswapV3Staker::claimRewardCall {
            rewardToken: incentive_key.reward_token,
            to: options.recipient,
            amountRequested: options.amount.unwrap_or_default(),
        }
        .abi_encode(),
    ]
}

/// Collect rewards from multiple programs at once.
///
/// Note:  A `tokenId` can be staked in many programs but to claim rewards and continue the program you must unstake, claim, and then restake.
/// You can only specify one amount and one recipient across the various programs if you are collecting from multiple programs at once.
///
/// ## Arguments
///
/// * `incentive_keys`: An array of IncentiveKeys that `tokenId` is staked in.
/// * `options`: ClaimOptions to specify tokenId, recipient, and amount wanting to collect.
///
pub fn collect_rewards<P>(
    incentive_keys: &[IncentiveKey<P>],
    options: ClaimOptions,
) -> MethodParameters {
    let mut calldatas = Vec::new();

    for incentive_key in incentive_keys {
        // unstakes and claims for the unique program
        calldatas.extend(encode_claim(incentive_key, options));
        // re-stakes the position for the unique program
        calldatas.push(
            IUniswapV3Staker::stakeTokenCall {
                key: encode_incentive_key(incentive_key),
                tokenId: options.token_id,
            }
            .abi_encode(),
        );
    }
    MethodParameters {
        calldata: encode_multicall(calldatas),
        value: U256::ZERO,
    }
}

/// Unstake, claim, and withdraw a position from multiple programs at once.
///
/// ## Arguments
///
/// * `incentive_keys`: A list of incentiveKeys to unstake from. Should include all incentiveKeys (unique staking programs) that `options.tokenId` is staked in.
/// * `withdraw_options`: Options for producing claim calldata and withdraw calldata. Can't withdraw without unstaking all programs for `tokenId`.
///
pub fn withdraw_token<P>(
    incentive_keys: &[IncentiveKey<P>],
    withdraw_options: FullWithdrawOptions,
) -> MethodParameters {
    let mut calldatas = Vec::new();

    for incentive_key in incentive_keys {
        // unstakes and claims for the unique program
        calldatas.extend(encode_claim(incentive_key, withdraw_options.claim_options));
    }
    let owner = withdraw_options.withdraw_options.owner;
    calldatas.push(
        IUniswapV3Staker::withdrawTokenCall {
            tokenId: withdraw_options.claim_options.token_id,
            to: owner,
            data: withdraw_options.withdraw_options.data.unwrap_or_default(),
        }
        .abi_encode(),
    );
    MethodParameters {
        calldata: encode_multicall(calldatas),
        value: U256::ZERO,
    }
}

pub fn encode_deposit<P>(incentive_keys: &[IncentiveKey<P>]) -> Vec<u8> {
    if incentive_keys.len() == 1 {
        encode_incentive_key(&incentive_keys[0]).abi_encode()
    } else {
        incentive_keys
            .iter()
            .map(encode_incentive_key)
            .collect::<Vec<_>>()
            .abi_encode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;
    use alloy_primitives::{hex, uint};
    use once_cell::sync::Lazy;
    use uniswap_sdk_core::{prelude::*, token};

    static REWARD: Lazy<Token> = Lazy::new(|| {
        token!(
            1,
            "1f9840a85d5aF5bf1D1762F925BDADdC4201F984",
            18,
            "r",
            "reward"
        )
    });

    static INCENTIVE_KEY: Lazy<IncentiveKey<NoTickDataProvider>> = Lazy::new(|| IncentiveKey {
        reward_token: REWARD.address(),
        pool: POOL_0_1.clone(),
        start_time: uint!(100_U256),
        end_time: uint!(200_U256),
        refundee: address!("0000000000000000000000000000000000000001"),
    });
    static INCENTIVE_KEYS: Lazy<Vec<IncentiveKey<NoTickDataProvider>>> = Lazy::new(|| {
        vec![
            INCENTIVE_KEY.clone(),
            IncentiveKey {
                reward_token: REWARD.address(),
                pool: POOL_0_1.clone(),
                start_time: uint!(50_U256),
                end_time: uint!(100_U256),
                refundee: address!("0000000000000000000000000000000000000089"),
            },
        ]
    });
    const RECIPIENT: Address = address!("0000000000000000000000000000000000000003");
    const SENDER: Address = address!("0000000000000000000000000000000000000004");
    const TOKEN_ID: U256 = uint!(1_U256);
    static WITHDRAW_OPTIONS: Lazy<FullWithdrawOptions> = Lazy::new(|| FullWithdrawOptions {
        claim_options: ClaimOptions {
            token_id: TOKEN_ID,
            recipient: RECIPIENT,
            amount: Some(U256::ZERO),
        },
        withdraw_options: WithdrawOptions {
            owner: SENDER,
            data: Some(hex!("0000000000000000000000000000000000000008").to_vec()),
        },
    });

    #[test]
    fn test_collect_rewards_succeeds_with_amount() {
        let options = ClaimOptions {
            token_id: TOKEN_ID,
            recipient: RECIPIENT,
            amount: Some(uint!(1_U256)),
        };
        let MethodParameters { calldata, value } =
            collect_rewards(&[INCENTIVE_KEY.clone()], options);
        assert_eq!(value, U256::ZERO);
        assert_eq!(
            calldata,
            hex!("ac9650d80000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c4f2d2909b0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c80000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000"),
        );
    }

    #[test]
    fn test_collect_rewards_succeeds_no_amount() {
        let options = ClaimOptions {
            token_id: TOKEN_ID,
            recipient: RECIPIENT,
            amount: None,
        };
        let MethodParameters { calldata, value } =
            collect_rewards(&[INCENTIVE_KEY.clone()], options);
        assert_eq!(value, U256::ZERO);
        assert_eq!(
            calldata,
            hex!("ac9650d80000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c4f2d2909b0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c80000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn test_collect_rewards_succeeds_multiple_keys() {
        let options = ClaimOptions {
            token_id: TOKEN_ID,
            recipient: RECIPIENT,
            amount: None,
        };
        let MethodParameters { calldata, value } = collect_rewards(&INCENTIVE_KEYS, options);
        assert_eq!(value, U256::ZERO);
        assert_eq!(
            calldata,
            hex!("ac9650d80000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001c0000000000000000000000000000000000000000000000000000000000000026000000000000000000000000000000000000000000000000000000000000003600000000000000000000000000000000000000000000000000000000000000460000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c4f2d2909b0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd200000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000064000000000000000000000000000000000000000000000000000000000000008900000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c4f2d2909b0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000003200000000000000000000000000000000000000000000000000000000000000640000000000000000000000000000000000000000000000000000000000000089000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn test_withdraw_token_succeeds_with_one_key() {
        let options = WITHDRAW_OPTIONS.clone();
        let MethodParameters { calldata, value } =
            withdraw_token(&[INCENTIVE_KEY.clone()], options);
        assert_eq!(value, U256::ZERO);
        assert_eq!(
            calldata,
            hex!("ac9650d80000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a43c423f0b0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn test_withdraw_token_succeeds_with_multiple_keys() {
        let options = WITHDRAW_OPTIONS.clone();
        let MethodParameters { calldata, value } = withdraw_token(&INCENTIVE_KEYS, options);
        assert_eq!(value, U256::ZERO);
        assert_eq!(
            calldata,
            hex!("ac9650d80000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000500000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000001a00000000000000000000000000000000000000000000000000000000000000240000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000003e000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c4f549ab420000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd200000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000064000000000000000000000000000000000000000000000000000000000000008900000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000642f2d783d0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f984000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a43c423f0b0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn test_encode_deposit_succeeds_single_key() {
        let deposit = encode_deposit(&[INCENTIVE_KEY.clone()]);
        assert_eq!(
            deposit,
            hex!("0000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c80000000000000000000000000000000000000000000000000000000000000001")
        );
    }

    #[test]
    fn test_encode_deposit_succeeds_multiple_keys() {
        let deposit = encode_deposit(&INCENTIVE_KEYS);
        assert_eq!(
            deposit,
            hex!("000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000020000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c800000000000000000000000000000000000000000000000000000000000000010000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000003200000000000000000000000000000000000000000000000000000000000000640000000000000000000000000000000000000000000000000000000000000089")
        );
    }

    #[test]
    fn test_safe_transfer_from_succeeds() {
        let data = encode_deposit(&[INCENTIVE_KEY.clone()]);
        let MethodParameters { calldata, value } =
            safe_transfer_from_parameters(SafeTransferOptions {
                sender: SENDER,
                recipient: RECIPIENT,
                token_id: TOKEN_ID,
                data,
            });
        assert_eq!(value, U256::ZERO);
        assert_eq!(
            calldata,
            hex!("b88d4fde000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000001f9840a85d5af5bf1d1762f925bdaddc4201f9840000000000000000000000004fa63b0dea87d2cd519f3b67a5ddb145779b7bd2000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c80000000000000000000000000000000000000000000000000000000000000001")
        );
    }
}

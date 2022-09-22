// used by crfa in prod, c1 key

use pallas::ledger::traverse::MultiEraOutput;
use pallas::ledger::traverse::{MultiEraBlock, OutputRef};
use serde::Deserialize;

use crate::crosscut::epochs::block_epoch;
use crate::{crosscut, model, prelude::*};

#[derive(Deserialize, Copy, Clone, PartialEq)]
pub enum AggrType {
    Epoch,
}

#[derive(Deserialize, Copy, Clone, PartialEq)]
pub enum ReportingMode {
    Volume,
}

#[derive(Deserialize, Clone)]
pub struct Config {
    pub key_prefix: Option<String>,
    pub filter: Option<crosscut::filters::Predicate>,
    pub aggr_by: Option<AggrType>,
    pub report_mode: Option<ReportingMode>,
}

pub struct Reducer {
    config: Config,
    policy: crosscut::policies::RuntimePolicy,
    chain: crosscut::ChainWellKnownInfo,
}

impl Reducer {

    fn config_key(&self, address: String, epoch_no: u64) -> String {
        let def_key_prefix = "balance_by_script";

        match &self.config.aggr_by {
            Some(aggr_type) if matches!(aggr_type, AggrType::Epoch) => {
                return match &self.config.key_prefix {
                    Some(prefix) => format!("{}.{}.{}", prefix, address, epoch_no),
                    None => format!("{}.{}", def_key_prefix.to_string(), address),
                };
            },
            _ => {
                return match &self.config.key_prefix {
                    Some(prefix) => format!("{}.{}", prefix, address),
                    None => format!("{}.{}", def_key_prefix.to_string(), address),
                };
            }
        };
    }

    fn process_inbound_txo(
        &mut self,
        ctx: &model::BlockContext,
        input: &OutputRef,
        output: &mut super::OutputPort,
        epoch_no: u64
    ) -> Result<(), gasket::error::Error> {

        let utxo = ctx.find_utxo(input).apply_policy(&self.policy).or_panic()?;

        let utxo = match utxo {
            Some(x) => x,
            None => return Ok(())
        };

        let is_script_address = utxo.address().map_or(false, |addr| addr.has_script());

        if !is_script_address {
            return Ok(());
        }

        let address = utxo.address().map(|addr| addr.to_string()).or_panic()?;

        let key = self.config_key(address, epoch_no);

        let amount: i64 = match &self.config.report_mode {
            Some(rep_mode) if matches!(rep_mode, ReportingMode::Volume) => utxo.ada_amount() as i64,
            _ => (-1) * utxo.ada_amount() as i64,
        };

        let crdt = model::CRDTCommand::PNCounter(key, amount);

        output.send(gasket::messaging::Message::from(crdt))?;

        Ok(())
    }

    fn process_outbound_txo(
        &mut self,
        tx_output: &MultiEraOutput,
        output: &mut super::OutputPort,
        epoch_no: u64,
    ) -> Result<(), gasket::error::Error> {
        let is_script_address = tx_output.address().map_or(false, |addr| addr.has_script());

        if !is_script_address {
            return Ok(());
        }

        let address = tx_output.address().map(|addr| addr.to_string()).or_panic()?;

        let key = self.config_key(address, epoch_no);

        let crdt = model::CRDTCommand::PNCounter(key, tx_output.ada_amount() as i64);

        output.send(gasket::messaging::Message::from(crdt))?;

        Ok(())
    }

    pub fn reduce_block<'b>(
        &mut self,
        block: &'b MultiEraBlock<'b>,
        ctx: &model::BlockContext,
        output: &mut super::OutputPort,
    ) -> Result<(), gasket::error::Error> {

        for tx in block.txs().into_iter() {
            if filter_matches!(self, block, &tx, ctx) {
                let epoch_no = block_epoch(&self.chain, block);

                for consumed in tx.consumes().iter().map(|i| i.output_ref()) {
                    self.process_inbound_txo(&ctx, &consumed, output, epoch_no)?;
                }
    
                for (_, produced) in tx.produces() {
                    self.process_outbound_txo(&produced, output, epoch_no)?;
                }
            }
        }

        Ok(())
    }
}

impl Config {
    pub fn plugin(self, 
        chain: &crosscut::ChainWellKnownInfo,
        policy: &crosscut::policies::RuntimePolicy) -> super::Reducer {
        let reducer = Reducer {
            config: self,
            policy: policy.clone(),
            chain: chain.clone(),
        };

        super::Reducer::BalanceByScript(reducer)
    }
}

use stratamoto::stratamoto_main;
use stratamoto_scenarios::{
    pool_setup_connection::PoolSetupConnectionScenario, setup_connection::TestCase,
};

stratamoto_main!(PoolSetupConnectionScenario, TestCase);

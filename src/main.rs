//! This example implements a TCP echo server on port 1234 and using DHCP.
//! Send it some data, you should see it echoed back and printed in the console.
//!
//! Example written for the [`WIZnet W5500-EVB-Pico`](https://www.wiznet.io/product-item/w5500-evb-pico/) board.

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::yield_now;
use embassy_net::{DhcpConfig, Stack, StackResources};
use embassy_net_wiznet::chip::W5500;
use embassy_net_wiznet::*;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{Async, Config as SpiConfig, Spi};
use embassy_rp::watchdog::Watchdog;
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_io_async::Write;
use heapless::String;
use panic_probe as _;
use rand::RngCore;
use static_cell::StaticCell;

/// Trigger a `HardFault` via `udf` instruction.
///
/// This function may be used to as `defmt::panic_handler` to avoid double prints.
///
/// # Examples
///
/// ```
/// #[defmt::panic_handler]
/// fn panic() -> ! {
///     panic_probe::hard_fault();
/// }
/// ```
#[cfg(target_os = "none")]
pub fn hard_fault() -> ! {
    // If `UsageFault` is enabled, we disable that first, since otherwise `udf` will cause that
    // exception instead of `HardFault`.
    const SHCSR: *mut u32 = 0xE000ED24usize as _;
    const USGFAULTENA: usize = 18;

    unsafe {
        let mut shcsr = core::ptr::read_volatile(SHCSR);
        shcsr &= !(1 << USGFAULTENA);
        core::ptr::write_volatile(SHCSR, shcsr);
    }

    cortex_m::asm::udf();
}

#[embassy_executor::task]
async fn ethernet_task(
    runner: Runner<
        'static,
        W5500,
        ExclusiveDevice<Spi<'static, SPI0, Async>, Output<'static>, Delay>,
        Input<'static>,
        Output<'static>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Device<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let mut watchdog = Watchdog::new(p.WATCHDOG);
    let mut rng = RoscRng;
    let mut led = Output::new(p.PIN_25, Level::Low);

    led.set_high();

    info!("W5500 TCP echo server example started");

    Timer::after_secs(1).await;
    led.set_low();

    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 25_000_000;
    let (miso, mosi, clk) = (p.PIN_16, p.PIN_19, p.PIN_18);
    let spi = Spi::new(p.SPI0, clk, mosi, miso, p.DMA_CH0, p.DMA_CH1, spi_cfg);
    let cs = Output::new(p.PIN_17, Level::High);
    let w5500_int = Input::new(p.PIN_15, Pull::Up);
    let w5500_reset = Output::new(p.PIN_14, Level::High);

    // let mac_addr = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mac_addr = [0xa2, 0x74, 0xbb, 0x19, 0x6f, 0x8f];
    static STATE: StaticCell<State<8, 8>> = StaticCell::new();
    let state = STATE.init(State::<8, 8>::new());
    let (device, runner) = loop {
        let result = embassy_net_wiznet::new(
            mac_addr,
            state,
            ExclusiveDevice::new(spi, cs, Delay),
            w5500_int,
            w5500_reset,
        )
        .await;
        if let Ok(items) = result {
            break items;
        }

        info!("W5500 init failed, retrying...");
        Timer::after_secs(1).await;
        watchdog.trigger_reset();
        loop {}
    };

    unwrap!(spawner.spawn(ethernet_task(runner)));

    // Generate random seed
    let seed = rng.next_u64();

    // Init network stack
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        device,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    // Launch network task
    unwrap!(spawner.spawn(net_task(runner)));

    info!("Waiting for DHCP...");
    let mut dhcpcfg = DhcpConfig::default();
    dhcpcfg.hostname = Some(String::try_from("embassy-w5500").unwrap());
    stack.set_config_v4(embassy_net::ConfigV4::Dhcp(dhcpcfg));
    let cfg = wait_for_config(stack).await;
    let local_addr = cfg.address.address();
    info!("IP address: {:?}", local_addr);

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];
    loop {
        let mut socket = embassy_net::tcp::TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        led.set_low();
        info!("Listening on TCP:1234...");
        if let Err(e) = socket.accept(1234).await {
            warn!("accept error: {:?}", e);
            continue;
        }
        info!("Received connection from {:?}", socket.remote_endpoint());
        led.set_high();

        loop {
            let n = match socket.read(&mut buf).await {
                Ok(0) => {
                    warn!("read EOF");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    warn!("{:?}", e);
                    break;
                }
            };
            info!("rxd {}", core::str::from_utf8(&buf[..n]).unwrap());

            if let Err(e) = socket.write_all(&buf[..n]).await {
                warn!("write error: {:?}", e);
                break;
            }
        }
    }
}

async fn wait_for_config(stack: Stack<'static>) -> embassy_net::StaticConfigV4 {
    loop {
        if let Some(config) = stack.config_v4() {
            return config.clone();
        }
        yield_now().await;
    }
}

# rust-embedded-w5100-driver
Driver for the W5100 Ethernet+TCP controller in Rust

Connections:

WS5100(SCK, MOSI, MISO) -> RP2040(SPI0: SCK=GP18, TX=GP19, RX=GP16)
WS5100(CS) -> RP2040(phys. pin 22 = SPI0 CSn = GP17)
WS5100(RST) -> RP2040(phys. pin 27 = GPIO 21)
WS5100(INT) -> RP2040(phys. pin 26 = GPIO 20)

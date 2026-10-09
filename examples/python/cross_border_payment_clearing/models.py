import os
from typing import List, Optional
from pydantic import BaseModel, Field


class FxRateResponse(BaseModel):
    base_currency: str = Field(..., description="Base Currency ISO Code (e.g. USD)")
    target_currency: str = Field(..., description="Target Currency ISO Code (e.g. EUR)")
    rate: float = Field(..., description="Live FX exchange rate multiplier")
    spread_bps: int = Field(default=12, description="Institutional spread in basis points")
    expires_in_seconds: int = Field(default=60, description="Rate validity TTL in seconds")


class PaymentTransferRequest(BaseModel):
    sender_account: str = Field(..., description="Originating account ID (e.g. ACT-1100)")
    recipient_iban: str = Field(..., description="Target international IBAN (e.g. DE89370400440532013000)")
    amount: float = Field(..., gt=0, description="Transfer amount in source currency")
    source_currency: str = Field(default="USD", min_length=3, max_length=3, description="Source currency code (USD)")
    target_currency: str = Field(default="EUR", min_length=3, max_length=3, description="Target currency code (EUR)")
    country_code: str = Field(default="DE", description="Destination 2-letter ISO country code (e.g. DE, US, IR)")


class PaymentTransferResponse(BaseModel):
    transaction_id: str = Field(..., description="Globally unique payment tracking ID")
    status: str = Field(..., description="SETTLED | ESCROW_HELD | FROZEN_COMPLIANCE")
    settlement_rail: str = Field(..., description="FEDNOW_INSTANT | SEPA_INSTANT | INSTITUTIONAL_ESCROW | AML_HOLD")
    source_amount: float = Field(..., description="Original transaction amount")
    converted_amount: float = Field(..., description="Total settled amount in destination currency")
    applied_fx_rate: float = Field(..., description="Applied currency conversion rate")
    clearing_fee: float = Field(..., description="Clearing and network rail fee")
    estimated_arrival: str = Field(..., description="Estimated arrival or clearing timeline")
    aml_reference_code: Optional[str] = Field(None, description="Regulatory compliance reference code")

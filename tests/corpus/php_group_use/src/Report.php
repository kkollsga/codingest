<?php

namespace App;

use App\Models\User;
use App\Domain\{Billing\Invoice, Catalog\Product};
use Psr\Log\LoggerInterface as Logger;

function build_report(User $user, Invoice $invoice, Product $product): string
{
    $who = $user->displayName();
    $sum = $invoice->grandTotal();
    $tag = $product->shelfLabel();
    $gap = compute_missing_total($user);

    return $who . $sum . $tag . $gap;
}

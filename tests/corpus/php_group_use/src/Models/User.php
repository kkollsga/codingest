<?php

namespace App\Models;

interface Identified
{
    public function displayName(): string;
}

class User implements Identified
{
    const STATUS = "active";

    public function displayName(): string
    {
        return "root";
    }
}
